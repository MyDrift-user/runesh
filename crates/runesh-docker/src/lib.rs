//! Docker/Podman workload driver via bollard.
//!
//! Connects to the Docker daemon via Unix socket (Linux/macOS),
//! named pipe (Windows), or TCP with TLS.

use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
    RestartContainerOptions, StopContainerOptions,
};
use futures_util::StreamExt;

use runesh_workload::{
    RunResult, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot, WorkloadState,
    WorkloadType,
};

/// Docker workload driver.
pub struct DockerDriver {
    client: Docker,
}

impl DockerDriver {
    /// Connect to the local Docker daemon using platform defaults.
    pub fn connect() -> Result<Self, WorkloadError> {
        let client = Docker::connect_with_local_defaults()
            .map_err(|e| WorkloadError::DriverError(format!("docker connect: {e}")))?;
        Ok(Self { client })
    }

    fn map_state(state: &str) -> WorkloadState {
        match state {
            "running" => WorkloadState::Running,
            "exited" | "dead" => WorkloadState::Stopped,
            "paused" => WorkloadState::Paused,
            "created" | "restarting" => WorkloadState::Creating,
            _ => WorkloadState::Unknown,
        }
    }
}

#[async_trait]
impl WorkloadDriver for DockerDriver {
    fn driver_name(&self) -> &str {
        "docker"
    }

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError> {
        let opts = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };
        let containers = self
            .client
            .list_containers(Some(opts))
            .await
            .map_err(|e| WorkloadError::DriverError(format!("list: {e}")))?;

        Ok(containers
            .into_iter()
            .map(|c| {
                let state_str = c.state.as_deref().unwrap_or("unknown");
                Workload {
                    id: c.id.unwrap_or_default(),
                    name: c
                        .names
                        .and_then(|n| n.first().map(|s| s.trim_start_matches('/').to_string()))
                        .unwrap_or_default(),
                    workload_type: WorkloadType::Container,
                    state: Self::map_state(state_str),
                    cpu_cores: None,
                    memory_mb: None,
                    disk_gb: None,
                    image: c.image,
                    host: None,
                    ips: c
                        .network_settings
                        .and_then(|ns| {
                            ns.networks.map(|nets| {
                                nets.values()
                                    .filter_map(|n| n.ip_address.clone())
                                    .filter(|ip| !ip.is_empty())
                                    .collect()
                            })
                        })
                        .unwrap_or_default(),
                }
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Workload, WorkloadError> {
        let info = self
            .client
            .inspect_container(id, None)
            .await
            .map_err(|e| WorkloadError::NotFound(format!("{id}: {e}")))?;

        let state = info
            .state
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .map(|s| Self::map_state(s.as_ref()))
            .unwrap_or(WorkloadState::Unknown);

        Ok(Workload {
            id: info.id.unwrap_or_default(),
            name: info
                .name
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string(),
            workload_type: WorkloadType::Container,
            state,
            cpu_cores: None,
            memory_mb: info
                .host_config
                .as_ref()
                .and_then(|hc| hc.memory)
                .map(|m| (m / 1024 / 1024) as u64),
            disk_gb: None,
            image: info.config.as_ref().and_then(|c| c.image.clone()),
            host: None,
            ips: vec![],
        })
    }

    async fn create(&self, spec: &serde_json::Value) -> Result<Workload, WorkloadError> {
        let image = spec["image"]
            .as_str()
            .ok_or_else(|| WorkloadError::OperationFailed("missing 'image' in spec".into()))?;
        let name = spec["name"].as_str().unwrap_or("runesh-container");

        let config = Config {
            image: Some(image.to_string()),
            ..Default::default()
        };

        let opts = CreateContainerOptions {
            name: name.to_string(),
            platform: None,
        };

        let resp = self
            .client
            .create_container(Some(opts), config)
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("create: {e}")))?;

        self.get(&resp.id).await
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        self.client
            .start_container::<String>(id, None)
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("start {id}: {e}")))
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        self.client
            .stop_container(id, Some(StopContainerOptions { t: 10 }))
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("stop {id}: {e}")))
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        self.client
            .restart_container(id, Some(RestartContainerOptions { t: 10 }))
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("restart {id}: {e}")))
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        self.client
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("destroy {id}: {e}")))
    }

    async fn snapshot(&self, _id: &str, _name: &str) -> Result<WorkloadSnapshot, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use docker commit for container snapshots".into(),
        ))
    }

    async fn list_snapshots(&self, _id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        Ok(vec![])
    }

    async fn restore_snapshot(&self, _snapshot_id: &str) -> Result<(), WorkloadError> {
        Err(WorkloadError::NotSupported(
            "container snapshot restore not supported".into(),
        ))
    }

    async fn run_command(&self, id: &str, command: &[&str]) -> Result<RunResult, WorkloadError> {
        use bollard::exec::{CreateExecOptions, StartExecResults};

        let create_opts = CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(command.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        };

        let exec_instance = self
            .client
            .create_exec(id, create_opts)
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("create exec: {e}")))?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let StartExecResults::Attached { mut output, .. } = self
            .client
            .start_exec(&exec_instance.id, None)
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("start exec: {e}")))?
        {
            while let Some(Ok(msg)) = output.next().await {
                match msg {
                    bollard::container::LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }

        let inspect = self
            .client
            .inspect_exec(&exec_instance.id)
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("inspect exec: {e}")))?;

        Ok(RunResult {
            exit_code: inspect.exit_code.unwrap_or(-1) as i32,
            stdout,
            stderr,
        })
    }

    async fn logs(&self, id: &str, lines: u32) -> Result<String, WorkloadError> {
        let opts = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: lines.to_string(),
            ..Default::default()
        };

        let mut stream = self.client.logs(id, Some(opts));
        let mut output = String::new();

        while let Some(Ok(msg)) = stream.next().await {
            output.push_str(&String::from_utf8_lossy(&msg.into_bytes()));
        }

        Ok(output)
    }

    async fn resize(
        &self,
        id: &str,
        _cpu: Option<u32>,
        memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError> {
        if let Some(mem) = memory_mb {
            let update = bollard::container::UpdateContainerOptions::<String> {
                memory: Some((mem * 1024 * 1024) as i64),
                ..Default::default()
            };
            self.client
                .update_container(id, update)
                .await
                .map_err(|e| WorkloadError::OperationFailed(format!("resize {id}: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_mapping() {
        assert_eq!(DockerDriver::map_state("running"), WorkloadState::Running);
        assert_eq!(DockerDriver::map_state("exited"), WorkloadState::Stopped);
        assert_eq!(DockerDriver::map_state("paused"), WorkloadState::Paused);
        assert_eq!(DockerDriver::map_state("created"), WorkloadState::Creating);
        assert_eq!(DockerDriver::map_state("xyz"), WorkloadState::Unknown);
    }
}
