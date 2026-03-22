use crate::error::{Error, Result};
use crate::font::GlyphAtlas;
use crate::vertex::{DrawCommand, Vertex};
use windows::core::PCSTR;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const VS_SOURCE: &[u8] = b"
struct VS_INPUT {
    float2 pos : POSITION;
    float4 col : COLOR;
    float2 uv  : TEXCOORD;
};
struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR;
    float2 uv  : TEXCOORD;
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
    return output;
}
\0";

const PS_SOLID_SOURCE: &[u8] = b"
struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR;
    float2 uv  : TEXCOORD;
};
float4 main(PS_INPUT input) : SV_TARGET {
    return input.col;
}
\0";

const PS_TEXTURED_SOURCE: &[u8] = b"
Texture2D tex : register(t0);
SamplerState samp : register(s0);
struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR;
    float2 uv  : TEXCOORD;
};
float4 main(PS_INPUT input) : SV_TARGET {
    float alpha = tex.Sample(samp, input.uv).r;
    return float4(input.col.rgb, input.col.a * alpha);
}
\0";

#[repr(C)]
struct ConstantBuffer {
    screen_size: [f32; 2],
    _padding: [f32; 2],
}

pub(crate) struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    render_target: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    ps_solid: ID3D11PixelShader,
    ps_textured: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    constant_buffer: ID3D11Buffer,
    blend_state: ID3D11BlendState,
    sampler: ID3D11SamplerState,
    raster_state: ID3D11RasterizerState,
    font_texture: Option<ID3D11ShaderResourceView>,
    width: u32,
    height: u32,
}

impl Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let (device, context, swap_chain) = create_device_and_swap_chain(hwnd, width, height)?;
        let (vs_blob, vertex_shader) = compile_and_create_vs(&device)?;
        let ps_solid = compile_and_create_ps(&device, PS_SOLID_SOURCE, "ps_solid")?;
        let ps_textured = compile_and_create_ps(&device, PS_TEXTURED_SOURCE, "ps_textured")?;
        let input_layout = create_input_layout(&device, &vs_blob)?;
        let constant_buffer = create_constant_buffer(&device)?;
        let blend_state = create_blend_state(&device)?;
        let sampler = create_sampler(&device)?;
        let raster_state = create_rasterizer_state(&device)?;

        let mut renderer = Self {
            device,
            context,
            swap_chain,
            render_target: None,
            vertex_shader,
            ps_solid,
            ps_textured,
            input_layout,
            constant_buffer,
            blend_state,
            sampler,
            raster_state,
            font_texture: None,
            width,
            height,
        };

        renderer.create_render_target()?;
        Ok(renderer)
    }

    pub fn upload_font_atlas(&mut self, atlas: &GlyphAtlas) -> Result<()> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: atlas.width,
            Height: atlas.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8_UNORM,
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
            SysMemPitch: atlas.width,
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

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }
        self.render_target = None;
        unsafe {
            self.swap_chain
                .ResizeBuffers(
                    0,
                    width,
                    height,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|_| Error::Renderer {
                    message: "resize failed".into(),
                })?;
        }
        self.width = width;
        self.height = height;
        self.create_render_target()
    }

    pub fn begin_frame(&self) {
        let rt = self.render_target.as_ref().unwrap();
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
    }

    pub fn submit(
        &self,
        vertices: &[Vertex],
        indices: &[u32],
        commands: &[DrawCommand],
    ) -> Result<()> {
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
            match cmd {
                DrawCommand::Solid {
                    index_offset,
                    index_count,
                    ..
                } => unsafe {
                    self.context.PSSetShader(&self.ps_solid, None);
                    self.context.DrawIndexed(*index_count, *index_offset, 0);
                },
                DrawCommand::Textured {
                    index_offset,
                    index_count,
                    ..
                } => unsafe {
                    self.context.PSSetShader(&self.ps_textured, None);
                    if let Some(ref srv) = self.font_texture {
                        self.context
                            .PSSetShaderResources(0, Some(&[Some(srv.clone())]));
                    }
                    self.context.DrawIndexed(*index_count, *index_offset, 0);
                },
            }
        }

        Ok(())
    }

    pub fn end_frame(&self) -> Result<()> {
        unsafe {
            self.swap_chain
                .Present(1, DXGI_PRESENT(0))
                .ok()
                .map_err(|_| Error::Renderer {
                    message: "present failed".into(),
                })
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
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("COLOR"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 8,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("TEXCOORD"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 24,
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
