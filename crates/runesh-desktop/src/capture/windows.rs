//! Windows screen capture using DXGI Desktop Duplication API.
//!
//! This is the fastest and most efficient way to capture the screen on Windows 8+.

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

pub struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    output_dup: IDXGIOutputDuplication,
    width: u32,
    height: u32,
    staging_texture: Option<ID3D11Texture2D>,
}

// SAFETY: We only access DXGI resources from one thread at a time.
unsafe impl Send for DxgiCapturer {}

impl DxgiCapturer {
    pub fn new(display_id: u32) -> Result<Self, DesktopError> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()
                .map_err(|e| DesktopError::Capture(format!("CreateDXGIFactory1: {e}")))?;

            let adapter = factory
                .EnumAdapters1(0)
                .map_err(|e| DesktopError::Capture(format!("EnumAdapters1: {e}")))?;

            let output: IDXGIOutput = adapter
                .EnumOutputs(display_id)
                .map_err(|_| DesktopError::DisplayNotFound(display_id))?;

            let output_desc = output.GetDesc()
                .map_err(|e| DesktopError::Capture(format!("GetDesc: {e}")))?;

            let rect = output_desc.DesktopCoordinates;
            let width = (rect.right - rect.left) as u32;
            let height = (rect.bottom - rect.top) as u32;

            // Create D3D11 device
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;

            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| DesktopError::Capture(format!("D3D11CreateDevice: {e}")))?;

            let device = device.ok_or_else(|| DesktopError::Capture("No D3D11 device".into()))?;
            let context = context.ok_or_else(|| DesktopError::Capture("No D3D11 context".into()))?;

            // Create desktop duplication
            let output1: IDXGIOutput1 = output.cast()
                .map_err(|e| DesktopError::Capture(format!("IDXGIOutput1 cast: {e}")))?;

            let output_dup = output1
                .DuplicateOutput(&device)
                .map_err(|e| DesktopError::Capture(format!("DuplicateOutput: {e}")))?;

            Ok(Self {
                device,
                context,
                output_dup,
                width,
                height,
                staging_texture: None,
            })
        }
    }

    fn ensure_staging_texture(&mut self) -> Result<(), DesktopError> {
        if self.staging_texture.is_some() {
            return Ok(());
        }

        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        unsafe {
            let mut texture: Option<ID3D11Texture2D> = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .map_err(|e| DesktopError::Capture(format!("CreateTexture2D: {e}")))?;
            self.staging_texture = texture;
        }

        Ok(())
    }
}

impl ScreenCapture for DxgiCapturer {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        self.ensure_staging_texture()?;

        unsafe {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            self.output_dup
                .AcquireNextFrame(100, &mut frame_info, &mut resource)
                .map_err(|e| DesktopError::Capture(format!("AcquireNextFrame: {e}")))?;

            let resource = resource.ok_or_else(|| DesktopError::Capture("No resource".into()))?;
            let texture: ID3D11Texture2D = resource.cast()
                .map_err(|e| DesktopError::Capture(format!("Texture cast: {e}")))?;

            let staging = self.staging_texture.as_ref().unwrap();
            self.context.CopyResource(staging, &texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| DesktopError::Capture(format!("Map: {e}")))?;

            let row_pitch = mapped.RowPitch as usize;
            let expected_pitch = (self.width * 4) as usize;
            let mut data = Vec::with_capacity((self.width * self.height * 4) as usize);

            let src = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                row_pitch * self.height as usize,
            );

            for row in 0..self.height as usize {
                let row_start = row * row_pitch;
                data.extend_from_slice(&src[row_start..row_start + expected_pitch]);
            }

            self.context.Unmap(staging, 0);
            self.output_dup.ReleaseFrame()
                .map_err(|e| DesktopError::Capture(format!("ReleaseFrame: {e}")))?;

            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            Ok(CapturedFrame {
                width: self.width,
                height: self.height,
                data,
                timestamp,
            })
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
