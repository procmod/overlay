use crate::error::{Error, Result};
use crate::font::{GlyphAtlas, OUTLINE_PAD};
use crate::vertex::{CommandKind, DrawCommand, Vertex};
use windows::core::{HRESULT, PCSTR};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const VS_SOURCE: &[u8] = b"
struct VS_INPUT {
    float2 pos    : POSITION;
    float4 col    : COLOR;
    float2 uv     : TEXCOORD0;
    float2 params : TEXCOORD1;
};
struct PS_INPUT {
    float4 pos    : SV_POSITION;
    float4 col    : COLOR;
    float2 uv     : TEXCOORD0;
    float2 params : TEXCOORD1;
};
cbuffer cb : register(b0) {
    float2 screen_size;
};
PS_INPUT main(VS_INPUT input) {
    PS_INPUT output;
    float2 ndc = float2(
        input.pos.x / screen_size.x * 2.0 - 1.0,
        1.0 - input.pos.y / screen_size.y * 2.0
    );
    output.pos = float4(ndc, 0.0, 1.0);
    output.col = input.col;
    output.uv = input.uv;
    output.params = input.params;
    return output;
}
\0";

const PS_SOLID_SOURCE: &[u8] = b"
struct PS_INPUT {
    float4 pos    : SV_POSITION;
    float4 col    : COLOR;
    float2 uv     : TEXCOORD0;
    float2 params : TEXCOORD1;
};
float4 main(PS_INPUT input) : SV_TARGET {
    return input.col;
}
\0";

const PS_GLYPH_SOURCE: &[u8] = b"
Texture2D tex : register(t0);
SamplerState samp : register(s0);
struct PS_INPUT {
    float4 pos    : SV_POSITION;
    float4 col    : COLOR;
    float2 uv     : TEXCOORD0;
    float2 params : TEXCOORD1;
};
float4 main(PS_INPUT input) : SV_TARGET {
    float alpha = tex.Sample(samp, input.uv).r;
    return float4(input.col.rgb, input.col.a * alpha);
}
\0";

/// Outline shader.
///
/// The green channel holds the distance from the texel to the glyph contour, encoded so
/// that 1.0 is on the contour and 0.0 is `OUTLINE_PAD` texels away. `params` carries the
/// outline width in pixels and the atlas-to-screen scale, so the edge is antialiased over
/// one screen pixel at any font size.
fn ps_glyph_outline_source() -> Vec<u8> {
    format!(
        "
Texture2D tex : register(t0);
SamplerState samp : register(s0);
struct PS_INPUT {{
    float4 pos    : SV_POSITION;
    float4 col    : COLOR;
    float2 uv     : TEXCOORD0;
    float2 params : TEXCOORD1;
}};
float4 main(PS_INPUT input) : SV_TARGET {{
    float nearness = tex.Sample(samp, input.uv).g;
    float to_contour = (1.0 - nearness) * {pad:.1} * input.params.y;
    float alpha = saturate(input.params.x + 0.5 - to_contour);
    return float4(input.col.rgb, input.col.a * alpha);
}}
\0",
        pad = OUTLINE_PAD as f32
    )
    .into_bytes()
}

#[repr(C)]
struct ConstantBuffer {
    screen_size: [f32; 2],
    _padding: [f32; 2],
}

/// A driver reset, a GPU hang, or an adapter change destroys every D3D11 resource the
/// overlay holds. DXGI reports it through the call that failed, not through a callback.
fn signals_device_loss(code: HRESULT) -> bool {
    code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_RESET
}

/// Owns the graphics device and rebuilds it when the driver takes it away.
///
/// The device is held in an `Option` so the dead one is released before a replacement is
/// requested. Building a second device and swap chain for the same window while the first
/// is still alive is not reliable.
pub(crate) struct Renderer {
    gpu: Option<Gpu>,
    hwnd: HWND,
    width: u32,
    height: u32,
    lost: Option<u32>,
    resets: usize,
}

impl Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            gpu: Some(Gpu::new(hwnd, width, height)?),
            hwnd,
            width,
            height,
            lost: None,
            resets: 0,
        })
    }

    /// True once a device loss has been observed and before it has been repaired.
    pub fn device_lost(&self) -> bool {
        self.lost.is_some()
    }

    /// Number of successful device rebuilds since the last call.
    pub fn take_resets(&mut self) -> usize {
        std::mem::take(&mut self.resets)
    }

    /// Release the dead device and build a replacement.
    ///
    /// The caller re-uploads anything the new device needs, the font atlas above all.
    pub fn recover(&mut self) -> Result<()> {
        let reason = self.lost.unwrap_or_default();
        self.gpu = None;
        let gpu = Gpu::new(self.hwnd, self.width, self.height)
            .map_err(|_| Error::DeviceLost { reason })?;
        self.gpu = Some(gpu);
        self.lost = None;
        self.resets += 1;
        Ok(())
    }

    pub fn upload_font_atlas(&mut self, atlas: &GlyphAtlas) -> Result<()> {
        let gpu = self.gpu.as_mut().ok_or(Error::DeviceLost {
            reason: self.lost.unwrap_or_default(),
        })?;
        gpu.upload_font_atlas(atlas)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        if self.lost.is_some() {
            return Ok(());
        }
        self.absorb(|gpu| gpu.resize(width, height))
    }

    pub fn begin_frame(&mut self) -> Result<()> {
        self.absorb(|gpu| gpu.begin_frame())
    }

    pub fn submit(
        &mut self,
        vertices: &[Vertex],
        indices: &[u32],
        commands: &[DrawCommand],
    ) -> Result<()> {
        self.absorb(|gpu| gpu.submit(vertices, indices, commands))
    }

    pub fn end_frame(&mut self) -> Result<()> {
        self.absorb(|gpu| gpu.end_frame())
    }

    /// Run an operation against the device, recording a device loss instead of reporting it.
    ///
    /// A lost device is repaired on the next `begin_frame`, so the frame that noticed the
    /// loss is simply dropped rather than failing the caller's render loop.
    fn absorb(&mut self, operation: impl FnOnce(&mut Gpu) -> Result<()>) -> Result<()> {
        let reason = self.lost.unwrap_or_default();
        let gpu = self.gpu.as_mut().ok_or(Error::DeviceLost { reason })?;
        match operation(gpu) {
            Err(Error::DeviceLost { reason }) => {
                self.lost = Some(reason);
                Ok(())
            }
            other => other,
        }
    }
}

/// Every resource tied to one D3D11 device.
///
/// Grouped so a lost device can be released as a unit before a replacement is built.
struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    render_target: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    ps_solid: ID3D11PixelShader,
    ps_glyph: ID3D11PixelShader,
    ps_glyph_outline: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    constant_buffer: ID3D11Buffer,
    blend_state: ID3D11BlendState,
    sampler: ID3D11SamplerState,
    raster_state: ID3D11RasterizerState,
    font_texture: Option<ID3D11ShaderResourceView>,
    width: u32,
    height: u32,
}

impl Gpu {
    fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let (device, context, swap_chain) = create_device_and_swap_chain(hwnd, width, height)?;
        let (vs_blob, vertex_shader) = compile_and_create_vs(&device)?;
        let ps_solid = compile_and_create_ps(&device, PS_SOLID_SOURCE, "ps_solid")?;
        let ps_glyph = compile_and_create_ps(&device, PS_GLYPH_SOURCE, "ps_glyph")?;
        let ps_glyph_outline =
            compile_and_create_ps(&device, &ps_glyph_outline_source(), "ps_glyph_outline")?;
        let input_layout = create_input_layout(&device, &vs_blob)?;
        let constant_buffer = create_constant_buffer(&device)?;
        let blend_state = create_blend_state(&device)?;
        let sampler = create_sampler(&device)?;
        let raster_state = create_rasterizer_state(&device)?;

        let mut gpu = Self {
            device,
            context,
            swap_chain,
            render_target: None,
            vertex_shader,
            ps_solid,
            ps_glyph,
            ps_glyph_outline,
            input_layout,
            constant_buffer,
            blend_state,
            sampler,
            raster_state,
            font_texture: None,
            width,
            height,
        };

        gpu.create_render_target()?;
        Ok(gpu)
    }

    fn upload_font_atlas(&mut self, atlas: &GlyphAtlas) -> Result<()> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: atlas.width,
            Height: atlas.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };

        let init_data = D3D11_SUBRESOURCE_DATA {
            pSysMem: atlas.pixels.as_ptr() as *const _,
            SysMemPitch: atlas.width * 2,
            ..Default::default()
        };

        let texture: ID3D11Texture2D = unsafe {
            let mut tex = None;
            self.device
                .CreateTexture2D(&desc, Some(&init_data), Some(&mut tex))
                .map_err(|_| Error::RenderTarget)?;
            tex.unwrap()
        };

        let srv = unsafe {
            let mut srv = None;
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut srv))
                .map_err(|_| Error::RenderTarget)?;
            srv.unwrap()
        };

        self.font_texture = Some(srv);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }
        self.render_target = None;
        let resized = unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        };
        self.width = width;
        self.height = height;
        if let Err(error) = resized {
            if signals_device_loss(error.code()) {
                return Err(Error::DeviceLost {
                    reason: error.code().0 as u32,
                });
            }
            return Err(Error::Renderer {
                message: "resize failed".into(),
            });
        }
        self.create_render_target()
    }

    fn begin_frame(&self) -> Result<()> {
        let rt = self.render_target.as_ref().ok_or(Error::RenderTarget)?;
        let clear_color = [0.0f32, 0.0, 0.0, 0.0];
        unsafe {
            self.context.ClearRenderTargetView(rt, &clear_color);
            self.context
                .OMSetRenderTargets(Some(&[Some(rt.clone())]), None);

            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            self.context.RSSetViewports(Some(&[viewport]));
            self.context.RSSetState(Some(&self.raster_state));
            self.context
                .OMSetBlendState(Some(&self.blend_state), None, 0xffffffff);
            self.context.IASetInputLayout(Some(&self.input_layout));
            self.context
                .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));

            let cb_data = ConstantBuffer {
                screen_size: [self.width as f32, self.height as f32],
                _padding: [0.0; 2],
            };
            self.context.UpdateSubresource(
                &self.constant_buffer,
                0,
                None,
                &cb_data as *const _ as *const _,
                0,
                0,
            );
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.constant_buffer.clone())]));
        }
        Ok(())
    }

    fn submit(&self, vertices: &[Vertex], indices: &[u32], commands: &[DrawCommand]) -> Result<()> {
        if vertices.is_empty() {
            return Ok(());
        }

        let vb = create_vertex_buffer(&self.device, vertices)?;
        let ib = create_index_buffer(&self.device, indices)?;

        unsafe {
            let stride = std::mem::size_of::<Vertex>() as u32;
            let offset = 0u32;
            self.context
                .IASetVertexBuffers(0, 1, Some(&Some(vb)), Some(&stride), Some(&offset));
            self.context
                .IASetIndexBuffer(Some(&ib), DXGI_FORMAT_R32_UINT, 0);
        }

        for cmd in commands {
            unsafe {
                match cmd.kind {
                    CommandKind::Solid => self.context.PSSetShader(&self.ps_solid, None),
                    CommandKind::Glyph => {
                        self.context.PSSetShader(&self.ps_glyph, None);
                        self.bind_font_texture();
                    }
                    CommandKind::GlyphOutline => {
                        self.context.PSSetShader(&self.ps_glyph_outline, None);
                        self.bind_font_texture();
                    }
                }
                self.context
                    .DrawIndexed(cmd.index_count, cmd.index_offset, 0);
            }
        }

        Ok(())
    }

    fn end_frame(&self) -> Result<()> {
        let presented = unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) };
        if presented.is_ok() {
            return Ok(());
        }
        if signals_device_loss(presented) {
            return Err(Error::DeviceLost {
                reason: presented.0 as u32,
            });
        }
        Err(Error::Renderer {
            message: "present failed".into(),
        })
    }

    fn bind_font_texture(&self) {
        if let Some(ref srv) = self.font_texture {
            unsafe {
                self.context
                    .PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            }
        }
    }

    fn create_render_target(&mut self) -> Result<()> {
        let backbuffer: ID3D11Texture2D = unsafe {
            self.swap_chain
                .GetBuffer(0)
                .map_err(|_| Error::RenderTarget)?
        };
        let rtv = unsafe {
            let mut rtv = None;
            self.device
                .CreateRenderTargetView(&backbuffer, None, Some(&mut rtv))
                .map_err(|_| Error::RenderTarget)?;
            rtv.unwrap()
        };
        self.render_target = Some(rtv);
        Ok(())
    }
}

fn create_device_and_swap_chain(
    hwnd: HWND,
    width: u32,
    height: u32,
) -> Result<(ID3D11Device, ID3D11DeviceContext, IDXGISwapChain)> {
    let sc_desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC {
            Width: width,
            Height: height,
            RefreshRate: DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ..Default::default()
        },
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        OutputWindow: hwnd,
        Windowed: true.into(),
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        ..Default::default()
    };

    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    let mut device = None;
    let mut context = None;
    let mut swap_chain = None;

    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&sc_desc),
            Some(&mut swap_chain),
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|_| Error::DeviceCreation)?;
    }

    Ok((device.unwrap(), context.unwrap(), swap_chain.unwrap()))
}

fn compile_shader(source: &[u8], entry: &str, target: &str, name: &str) -> Result<Vec<u8>> {
    let entry_cstr = std::ffi::CString::new(entry).unwrap();
    let target_cstr = std::ffi::CString::new(target).unwrap();
    let name_cstr = std::ffi::CString::new(name).unwrap();

    let mut blob = None;
    let mut error_blob = None;

    let hr = unsafe {
        D3DCompile(
            source.as_ptr() as *const _,
            source.len() - 1, // exclude null terminator
            PCSTR(name_cstr.as_ptr() as *const _),
            None,
            None,
            PCSTR(entry_cstr.as_ptr() as *const _),
            PCSTR(target_cstr.as_ptr() as *const _),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut blob,
            Some(&mut error_blob),
        )
    };

    if hr.is_err() {
        let msg = if let Some(err) = error_blob {
            let ptr = unsafe { err.GetBufferPointer() } as *const u8;
            let len = unsafe { err.GetBufferSize() };
            let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
            String::from_utf8_lossy(bytes).to_string()
        } else {
            "unknown shader compilation error".to_string()
        };
        return Err(Error::ShaderCompilation { message: msg });
    }

    let blob = blob.unwrap();
    let ptr = unsafe { blob.GetBufferPointer() } as *const u8;
    let len = unsafe { blob.GetBufferSize() };
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn compile_and_create_vs(device: &ID3D11Device) -> Result<(Vec<u8>, ID3D11VertexShader)> {
    let bytecode = compile_shader(VS_SOURCE, "main", "vs_5_0", "vertex")?;
    let vs = unsafe {
        let mut vs = None;
        device
            .CreateVertexShader(&bytecode, None, Some(&mut vs))
            .map_err(|_| Error::ShaderCompilation {
                message: "failed to create vertex shader".into(),
            })?;
        vs.unwrap()
    };
    Ok((bytecode, vs))
}

fn compile_and_create_ps(
    device: &ID3D11Device,
    source: &[u8],
    name: &str,
) -> Result<ID3D11PixelShader> {
    let bytecode = compile_shader(source, "main", "ps_5_0", name)?;
    let ps = unsafe {
        let mut ps = None;
        device
            .CreatePixelShader(&bytecode, None, Some(&mut ps))
            .map_err(|_| Error::ShaderCompilation {
                message: format!("failed to create pixel shader: {name}"),
            })?;
        ps.unwrap()
    };
    Ok(ps)
}

fn create_input_layout(device: &ID3D11Device, vs_blob: &[u8]) -> Result<ID3D11InputLayout> {
    let descs = [
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("POSITION"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::offset_of!(Vertex, position) as u32,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("COLOR"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::offset_of!(Vertex, color) as u32,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("TEXCOORD"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::offset_of!(Vertex, uv) as u32,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("TEXCOORD"),
            SemanticIndex: 1,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::offset_of!(Vertex, params) as u32,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    let layout = unsafe {
        let mut layout = None;
        device
            .CreateInputLayout(&descs, vs_blob, Some(&mut layout))
            .map_err(|_| Error::Renderer {
                message: "failed to create input layout".into(),
            })?;
        layout.unwrap()
    };
    Ok(layout)
}

fn create_constant_buffer(device: &ID3D11Device) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: 16, // float2 + padding to 16-byte alignment
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        ..Default::default()
    };
    let buf = unsafe {
        let mut buf = None;
        device
            .CreateBuffer(&desc, None, Some(&mut buf))
            .map_err(|_| Error::Renderer {
                message: "failed to create constant buffer".into(),
            })?;
        buf.unwrap()
    };
    Ok(buf)
}

fn create_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D11_BLEND_SRC_ALPHA,
        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_ONE,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };

    let state = unsafe {
        let mut state = None;
        device
            .CreateBlendState(&desc, Some(&mut state))
            .map_err(|_| Error::Renderer {
                message: "failed to create blend state".into(),
            })?;
        state.unwrap()
    };
    Ok(state)
}

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        ..Default::default()
    };
    let sampler = unsafe {
        let mut sampler = None;
        device
            .CreateSamplerState(&desc, Some(&mut sampler))
            .map_err(|_| Error::Renderer {
                message: "failed to create sampler".into(),
            })?;
        sampler.unwrap()
    };
    Ok(sampler)
}

fn create_rasterizer_state(device: &ID3D11Device) -> Result<ID3D11RasterizerState> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        ScissorEnable: false.into(),
        DepthClipEnable: true.into(),
        ..Default::default()
    };
    let state = unsafe {
        let mut state = None;
        device
            .CreateRasterizerState(&desc, Some(&mut state))
            .map_err(|_| Error::Renderer {
                message: "failed to create rasterizer state".into(),
            })?;
        state.unwrap()
    };
    Ok(state)
}

fn create_vertex_buffer(device: &ID3D11Device, vertices: &[Vertex]) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of_val(vertices) as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
        ..Default::default()
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: vertices.as_ptr() as *const _,
        ..Default::default()
    };
    let buf = unsafe {
        let mut buf = None;
        device
            .CreateBuffer(&desc, Some(&init), Some(&mut buf))
            .map_err(|_| Error::Renderer {
                message: "failed to create vertex buffer".into(),
            })?;
        buf.unwrap()
    };
    Ok(buf)
}

fn create_index_buffer(device: &ID3D11Device, indices: &[u32]) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of_val(indices) as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_INDEX_BUFFER.0 as u32,
        ..Default::default()
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: indices.as_ptr() as *const _,
        ..Default::default()
    };
    let buf = unsafe {
        let mut buf = None;
        device
            .CreateBuffer(&desc, Some(&init), Some(&mut buf))
            .map_err(|_| Error::Renderer {
                message: "failed to create index buffer".into(),
            })?;
        buf.unwrap()
    };
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &[u8], target: &str, name: &str) {
        if let Err(error) = compile_shader(source, "main", target, name) {
            panic!("{name} failed to compile: {error}");
        }
    }

    // D3DCompile needs no device, so this runs anywhere the compiler DLL is present and
    // catches HLSL errors that would otherwise only surface when an overlay is created
    #[test]
    fn device_loss_is_told_apart_from_other_failures() {
        assert!(signals_device_loss(DXGI_ERROR_DEVICE_REMOVED));
        assert!(signals_device_loss(DXGI_ERROR_DEVICE_RESET));
        assert!(!signals_device_loss(DXGI_ERROR_INVALID_CALL));
        assert!(!signals_device_loss(HRESULT(0)));
    }

    #[test]
    fn shaders_compile() {
        compile(VS_SOURCE, "vs_5_0", "vertex");
        compile(PS_SOLID_SOURCE, "ps_5_0", "ps_solid");
        compile(PS_GLYPH_SOURCE, "ps_5_0", "ps_glyph");
        compile(&ps_glyph_outline_source(), "ps_5_0", "ps_glyph_outline");
    }
}
