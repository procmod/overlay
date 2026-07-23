use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("target window not found")]
    WindowNotFound,

    #[error("target process does not exist: {pid}")]
    ProcessNotFound { pid: u32 },

    #[error("process {pid} has no visible top-level window on the current desktop")]
    ProcessWindowNotFound { pid: u32 },

    #[error("overlay window closed")]
    OverlayClosed,

    #[error("target window was lost")]
    TargetWindowLost,

    #[error("failed to create overlay window")]
    WindowCreation(#[source] std::io::Error),

    #[error("failed to create D3D11 device")]
    DeviceCreation,

    #[error("failed to create swap chain")]
    SwapChainCreation,

    #[error("failed to compile shader: {message}")]
    ShaderCompilation { message: String },

    #[error("failed to create render target")]
    RenderTarget,

    #[error("renderer error: {message}")]
    Renderer { message: String },

    #[error("frame not in progress")]
    NoActiveFrame,
}

pub type Result<T> = std::result::Result<T, Error>;
