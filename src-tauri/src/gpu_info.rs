use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GpuVramInfo {
    pub adapter_name: String,
    pub dedicated_vram_bytes: u64,
    pub available_vram_bytes: u64,
}

#[cfg(target_os = "windows")]
pub fn get_primary_gpu_vram_info() -> Option<GpuVramInfo> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
        DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIFactory1,
    };
    use windows::core::Interface;

    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;

        let mut best: Option<GpuVramInfo> = None;
        let mut index = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(adapter) => adapter,
                Err(_) => break,
            };
            index += 1;

            let desc = match adapter.GetDesc1() {
                Ok(desc) => desc,
                Err(_) => continue,
            };

            // Skip the software/basic-render fallback adapter (Microsoft Basic Render Driver).
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }

            let adapter3: IDXGIAdapter3 = match adapter.cast() {
                Ok(adapter3) => adapter3,
                Err(_) => continue,
            };

            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if adapter3
                .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
                .is_err()
            {
                continue;
            }

            let name = String::from_utf16_lossy(
                &desc.Description[..desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len())],
            );

            let available = info.Budget.saturating_sub(info.CurrentUsage);
            let candidate = GpuVramInfo {
                adapter_name: name,
                dedicated_vram_bytes: desc.DedicatedVideoMemory as u64,
                available_vram_bytes: available,
            };

            // Prefer the adapter with the most dedicated VRAM (the discrete GPU, if any).
            let better = match &best {
                Some(current) => candidate.dedicated_vram_bytes > current.dedicated_vram_bytes,
                None => true,
            };
            if better {
                best = Some(candidate);
            }
        }

        best
    }
}

#[cfg(not(target_os = "windows"))]
pub fn get_primary_gpu_vram_info() -> Option<GpuVramInfo> {
    None
}

#[cfg(target_os = "windows")]
pub fn vulkan_runtime_available() -> bool {
    // whisper.cpp's Vulkan backend dynamically depends on the system Vulkan loader.
    // If it isn't present (missing/outdated GPU drivers), GPU mode can't work at all,
    // regardless of what hardware is installed.
    unsafe { libloading::Library::new("vulkan-1.dll") }.is_ok()
}

#[cfg(not(target_os = "windows"))]
pub fn vulkan_runtime_available() -> bool {
    // Best-effort: assume available elsewhere for now. The per-model VRAM check in
    // `check_gpu_vram` still guards against a GPU that's too weak for the selected model.
    true
}
