//! Windows screen capture via Windows.Graphics.Capture.
//!
//! Replaces DXGI `IDXGIOutputDuplication::DuplicateOutput`, which
//! returns `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` inside RDP sessions
//! and on some multi-GPU configurations. WGC is the Microsoft-
//! recommended capture path (used by OBS, Xbox Game Bar, and the
//! Teams screen-share stack) and works uniformly across console,
//! RDP, and Fast User Switching.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::Interface;

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

/// Screen capturer backed by `Windows.Graphics.Capture`.
///
/// Name kept as `DxgiCapturer` for drop-in compatibility with
/// `runesh_desktop::capture::create_capturer`'s dispatch. Underlying
/// implementation no longer uses `DuplicateOutput`.
pub struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    frame_pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    _item: GraphicsCaptureItem,
    frame_rx: Receiver<FrameTexture>,
    frame_arrived_token: i64,
    width: u32,
    height: u32,
    staging: Option<ID3D11Texture2D>,
    staging_dims: (u32, u32),
}

struct FrameTexture {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
}

// SAFETY: every COM pointer we hold is thread-safe under the Windows
// SDK's documented contract. We serialize all mutation through
// `&mut self` on `ScreenCapture::capture_frame`.
#[allow(unsafe_code)]
unsafe impl Send for DxgiCapturer {}

impl DxgiCapturer {
    pub fn new(display_id: u32) -> Result<Self, DesktopError> {
        if !GraphicsCaptureSession::IsSupported()
            .map_err(|e| DesktopError::Capture(format!("WGC IsSupported: {e}")))?
        {
            return Err(DesktopError::Capture(
                "Windows.Graphics.Capture not supported on this OS (Win10 1903+ required)".into(),
            ));
        }
        if display_id != 0 {
            return Err(DesktopError::DisplayNotFound(display_id));
        }

        // SAFETY: point (0, 0) is a valid input for MonitorFromPoint;
        // return value is a handle or NULL.
        #[allow(unsafe_code)]
        let monitor: HMONITOR =
            unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        if monitor.is_invalid() {
            return Err(DesktopError::DisplayNotFound(display_id));
        }

        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .map_err(|e| DesktopError::Capture(format!("WGC interop factory: {e}")))?;

        // SAFETY: `monitor` is a valid HMONITOR for the duration of the call.
        #[allow(unsafe_code)]
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForMonitor(monitor) }.map_err(|e| {
                // E_ACCESSDENIED = not in an interactive session (Session 0
                // LocalSystem service). Surface the dedicated variant so
                // callers can route through the session helper.
                if e.code().0 as u32 == 0x80070005 {
                    DesktopError::RequiresInteractiveSession
                } else {
                    DesktopError::Capture(format!("CreateForMonitor: {e}"))
                }
            })?;

        let size = item
            .Size()
            .map_err(|e| DesktopError::Capture(format!("WGC item size: {e}")))?;
        let width = size.Width as u32;
        let height = size.Height as u32;

        // 1. D3D11 device (hardware, BGRA support).
        let mut d3d_device: Option<ID3D11Device> = None;
        let mut d3d_context: Option<ID3D11DeviceContext> = None;
        // SAFETY: out-params outlive the call.
        #[allow(unsafe_code)]
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,
                Some(&mut d3d_context),
            )
        }
        .map_err(|e| DesktopError::Capture(format!("D3D11CreateDevice: {e}")))?;
        let device = d3d_device.ok_or_else(|| DesktopError::Capture("null D3D11 device".into()))?;
        let context =
            d3d_context.ok_or_else(|| DesktopError::Capture("null D3D11 context".into()))?;

        // 2. WinRT wrapper around the D3D11 device.
        let dxgi_device: IDXGIDevice = device
            .cast()
            .map_err(|e| DesktopError::Capture(format!("IDXGIDevice cast: {e}")))?;
        // SAFETY: `dxgi_device` is a valid IDXGIDevice for the call.
        #[allow(unsafe_code)]
        let inspectable =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }.map_err(|e| {
                DesktopError::Capture(format!("CreateDirect3D11DeviceFromDXGIDevice: {e}"))
            })?;
        let winrt_device: IDirect3DDevice = inspectable
            .cast()
            .map_err(|e| DesktopError::Capture(format!("IDirect3DDevice cast: {e}")))?;

        // 3. Free-threaded frame pool (no DispatcherQueue needed).
        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(|e| DesktopError::Capture(format!("CreateFramePool: {e}")))?;

        // 4. FrameArrived → bounded channel. Capacity 2 gives us a
        //    one-frame rendezvous plus an in-flight slot without
        //    letting the queue grow unbounded.
        let (tx, frame_rx) = sync_channel::<FrameTexture>(2);
        let tx_shared: Arc<Mutex<Option<SyncSender<FrameTexture>>>> =
            Arc::new(Mutex::new(Some(tx)));
        let tx_handler = tx_shared.clone();
        let frame_arrived_token = frame_pool
            .FrameArrived(&TypedEventHandler::new(
                move |pool: windows::core::Ref<Direct3D11CaptureFramePool>,
                      _args: windows::core::Ref<windows::core::IInspectable>|
                      -> windows::core::Result<()> {
                    let Some(pool) = pool.as_ref() else {
                        return Ok(());
                    };
                    let frame = pool.TryGetNextFrame()?;
                    let surface = frame.Surface()?;
                    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
                    // SAFETY: `access` is a valid IDirect3DDxgiInterfaceAccess.
                    #[allow(unsafe_code)]
                    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }?;
                    let content_size = frame.ContentSize().unwrap_or_default();
                    let ft = FrameTexture {
                        texture,
                        width: content_size.Width as u32,
                        height: content_size.Height as u32,
                    };
                    if let Ok(guard) = tx_handler.lock()
                        && let Some(tx) = guard.as_ref()
                    {
                        // Non-blocking: drop on backpressure.
                        let _ = tx.try_send(ft);
                    }
                    Ok(())
                },
            ))
            .map_err(|e| DesktopError::Capture(format!("FrameArrived: {e}")))?;

        // 5. Start the session.
        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(|e| DesktopError::Capture(format!("CreateCaptureSession: {e}")))?;
        // Suppress the Windows 11 yellow capture border when available.
        // No-ops on earlier Windows versions.
        let _ = session.SetIsBorderRequired(false);
        // Include the system cursor; the whole point of remote desktop.
        let _ = session.SetIsCursorCaptureEnabled(true);
        session
            .StartCapture()
            .map_err(|e| DesktopError::Capture(format!("StartCapture: {e}")))?;

        Ok(Self {
            device,
            context,
            frame_pool,
            _session: session,
            _item: item,
            frame_rx,
            frame_arrived_token,
            width,
            height,
            staging: None,
            staging_dims: (0, 0),
        })
    }

    fn ensure_staging(&mut self, w: u32, h: u32) -> Result<(), DesktopError> {
        if self.staging.is_some() && self.staging_dims == (w, h) {
            return Ok(());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        // SAFETY: `desc` outlives the call; `tex` is an out-param.
        #[allow(unsafe_code)]
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex)) }
            .map_err(|e| DesktopError::Capture(format!("staging CreateTexture2D: {e}")))?;
        self.staging = tex;
        self.staging_dims = (w, h);
        Ok(())
    }
}

impl Drop for DxgiCapturer {
    fn drop(&mut self) {
        let _ = self.frame_pool.RemoveFrameArrived(self.frame_arrived_token);
        let _ = self.frame_pool.Close();
    }
}

/// Secure-desktop guard: returns true only when the current input
/// desktop is the user's "Default" (i.e. NOT the UAC / lock screen /
/// Ctrl-Alt-Del secure desktop). Same check the DXGI path used.
fn is_user_desktop_active() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::StationsAndDesktops::{
        DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, GetUserObjectInformationW, OpenInputDesktop,
        UOI_NAME,
    };
    // SAFETY: all inputs are const enums / 0 flags; out-params sized correctly.
    #[allow(unsafe_code)]
    unsafe {
        let desktop = match OpenInputDesktop(
            DESKTOP_CONTROL_FLAGS(0),
            false,
            DESKTOP_ACCESS_FLAGS(0x0100),
        ) {
            Ok(d) => d,
            Err(_) => return false,
        };
        let handle = HANDLE(desktop.0);
        let mut needed: u32 = 0;
        let _ = GetUserObjectInformationW(handle, UOI_NAME, None, 0, Some(&mut needed));
        if needed == 0 {
            return false;
        }
        let mut buf: Vec<u16> = vec![0u16; (needed as usize).div_ceil(2) + 1];
        let raw = buf.as_mut_ptr() as *mut core::ffi::c_void;
        let ok = GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(raw),
            buf.len() as u32 * 2,
            Some(&mut needed),
        );
        if ok.is_err() {
            return false;
        }
        let nul = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let name = String::from_utf16_lossy(&buf[..nul]);
        name.eq_ignore_ascii_case("Default")
    }
}

impl ScreenCapture for DxgiCapturer {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        if !is_user_desktop_active() {
            return Err(DesktopError::Capture(
                "secure desktop active; capture suppressed".into(),
            ));
        }

        // Block for the next frame. Two-second ceiling so a
        // pathologically idle desktop returns a recoverable error
        // instead of deadlocking.
        let wgc = self
            .frame_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| DesktopError::Capture("WGC frame timeout".into()))?;
        self.ensure_staging(wgc.width, wgc.height)?;
        let staging = self.staging.as_ref().unwrap();

        // SAFETY: `staging` and `wgc.texture` are live D3D11 textures;
        // `self.context` is a valid D3D11 context for this thread.
        #[allow(unsafe_code)]
        unsafe {
            self.context.CopyResource(staging, &wgc.texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| DesktopError::Capture(format!("Map: {e}")))?;
            let row_pitch = mapped.RowPitch as usize;
            let expected = (wgc.width * 4) as usize;
            if mapped.RowPitch < wgc.width * 4 {
                self.context.Unmap(staging, 0);
                return Err(DesktopError::Capture(format!(
                    "row pitch {} < width*4 {}",
                    mapped.RowPitch,
                    wgc.width * 4
                )));
            }
            let src_len = row_pitch
                .checked_mul(wgc.height as usize)
                .ok_or_else(|| DesktopError::Capture("row pitch overflow".into()))?;
            let src = std::slice::from_raw_parts(mapped.pData as *const u8, src_len);
            let mut data = Vec::with_capacity((wgc.width * wgc.height * 4) as usize);
            for row in 0..wgc.height as usize {
                let start = row * row_pitch;
                data.extend_from_slice(&src[start..start + expected]);
            }
            self.context.Unmap(staging, 0);

            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            Ok(CapturedFrame {
                width: wgc.width,
                height: wgc.height,
                data,
                timestamp,
            })
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
