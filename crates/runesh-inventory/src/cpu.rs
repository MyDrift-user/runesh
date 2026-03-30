//! CPU information collection using sysinfo.

use sysinfo::System;

use crate::models::{CoreUsage, CpuInfo};

/// Collect CPU information from the system.
pub fn collect_cpu(sys: &System) -> CpuInfo {
    let cpus = sys.cpus();
    let global_usage = sys.global_cpu_usage();

    let brand = cpus.first().map(|c| c.brand().to_string()).unwrap_or_default();
    let vendor = cpus.first().map(|c| c.vendor_id().to_string()).unwrap_or_default();
    let frequency = cpus.first().map(|c| c.frequency()).unwrap_or(0);

    let per_core_usage: Vec<CoreUsage> = cpus
        .iter()
        .enumerate()
        .map(|(i, cpu)| CoreUsage {
            core_id: i,
            usage_percent: cpu.cpu_usage(),
            frequency_mhz: cpu.frequency(),
        })
        .collect();

    let physical_cores = System::physical_core_count().unwrap_or(cpus.len());

    CpuInfo {
        brand,
        vendor,
        physical_cores,
        logical_cores: cpus.len(),
        frequency_mhz: frequency,
        usage_percent: global_usage,
        per_core_usage,
    }
}
