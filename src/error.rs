use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("target window not found")]
    WindowNotFound,

    #[error("target window closed")]
    WindowClosed,

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
