use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub recommends_sm5: bool,
    pub reason: Option<String>,
}

pub fn detect_gpu_info() -> GpuInfo {
    let lspci_output = Command::new("lspci")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_default();

    let mut gpu_line = String::new();
    for line in lspci_output.lines() {
        let upper = line.to_uppercase();
        if upper.contains("VGA COMPATIBLE") || upper.contains("3D CONTROLLER") {
            gpu_line = line.to_string();
            break;
        }
    }

    if gpu_line.is_empty() {
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let uevent_path = entry.path().join("device/uevent");
                if let Ok(content) = std::fs::read_to_string(uevent_path) {
                    if content.contains("DRIVER=") && content.contains("PCI_ID=") {
                        gpu_line = content;
                        break;
                    }
                }
            }
        }
    }

    let upper = gpu_line.to_uppercase();

    let vendor = if upper.contains("AMD")
        || upper.contains("ADVANCED MICRO")
        || upper.contains("1002:")
        || upper.contains("DRIVER=AMDGPU")
        || upper.contains("DRIVER=RADEON")
    {
        "AMD"
    } else if upper.contains("NVIDIA") || upper.contains("10DE:") || upper.contains("DRIVER=NVIDIA") {
        "NVIDIA"
    } else if upper.contains("INTEL")
        || upper.contains("8086:")
        || upper.contains("DRIVER=I915")
        || upper.contains("DRIVER=XE")
    {
        "Intel"
    } else {
        "Outro"
    };

    let is_polaris = upper.contains("ELLESMERE")
        || upper.contains("POLARIS")
        || upper.contains("BAFFIN")
        || upper.contains("LEXA")
        || upper.contains("67DF")
        || upper.contains("67C0")
        || upper.contains("67EF")
        || upper.contains("67FF")
        || upper.contains("RX 470")
        || upper.contains("RX 480")
        || upper.contains("RX 570")
        || upper.contains("RX 580")
        || upper.contains("RX 590");

    let is_legacy_amd = is_polaris
        || upper.contains("VEGA")
        || upper.contains("HAWAII")
        || upper.contains("TONGA")
        || upper.contains("FIJI");

    let is_legacy_intel = upper.contains("HD GRAPHICS") || upper.contains("UHD GRAPHICS 6");

    let is_legacy_nvidia =
        upper.contains("GTX 9") || upper.contains("GTX 7") || upper.contains("GTX 6");

    let (recommends_sm5, reason) = if is_polaris {
        (
            true,
            Some("GPU AMD Polaris (RX 400/500) detectada. Vulkan SM5 é fortemente recomendado para evitar falhas de shader e travamentos no Linux.".to_string()),
        )
    } else if is_legacy_amd {
        (
            true,
            Some("GPU AMD GCN/Vega detectada. Vulkan SM5 é recomendado para garantir estabilidade e evitar falhas de shader no Linux.".to_string()),
        )
    } else if is_legacy_intel {
        (
            true,
            Some("GPU Intel integrada legada detectada. Vulkan SM5 é recomendado para evitar erros de renderização.".to_string()),
        )
    } else if is_legacy_nvidia {
        (
            true,
            Some("GPU NVIDIA de geração anterior detectada. Vulkan SM5 é recomendado para maior estabilidade no Linux.".to_string()),
        )
    } else {
        (false, None)
    };

    let display_name = if !gpu_line.is_empty() && gpu_line.contains("controller:") {
        gpu_line
            .split("controller:")
            .nth(1)
            .unwrap_or(&gpu_line)
            .trim()
            .to_string()
    } else if !gpu_line.is_empty() {
        format!("{vendor} Graphics")
    } else {
        "Dispositivo gráfico não identificado".to_string()
    };

    GpuInfo {
        name: display_name,
        vendor: vendor.to_string(),
        recommends_sm5,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_gpu_info() {
        let info = detect_gpu_info();
        println!("GPU detectada: {:?}", info);
        assert!(!info.vendor.is_empty());
    }
}
