//! Running process snapshot collection.

use sysinfo::System;

use crate::models::ProcessInfo;

/// Collect information about all running processes.
pub fn collect_processes(sys: &System) -> Vec<ProcessInfo> {
    sys.processes()
        .iter()
        .map(|(pid, proc_info)| {
            let status = format!("{:?}", proc_info.status());
            ProcessInfo {
                pid: pid.as_u32(),
                name: proc_info.name().to_string_lossy().to_string(),
                exe_path: proc_info
                    .exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                cmd: proc_info
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect(),
                status,
                cpu_usage: proc_info.cpu_usage(),
                memory_bytes: proc_info.memory(),
                user: proc_info.user_id().map(|u| u.to_string()),
                start_time: proc_info.start_time(),
                parent_pid: proc_info.parent().map(|p| p.as_u32()),
            }
        })
        .collect()
}
