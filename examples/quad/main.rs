#![cfg_attr(
    not(any(
        feature = "vulkan",
        feature = "dx11",
        feature = "dx12",
        feature = "metal",
        feature = "gl",
        feature = "wgl"
    )),
    allow(dead_code, unused_extern_crates, unused_imports)
)]

#[cfg(feature = "dx11")]
extern crate gfx_backend_dx11 as back;
#[cfg(feature = "dx12")]
extern crate gfx_backend_dx12 as back;
#[cfg(any(feature = "gl", feature = "wgl"))]
extern crate gfx_backend_gl as back;
#[cfg(feature = "metal")]
extern crate gfx_backend_metal as back;
#[cfg(feature = "vulkan")]
extern crate gfx_backend_vulkan as back;
extern crate gfx_hal as hal;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    main();
}

use hal::format::{AsFormat, ChannelType, /*Rgba8Srgb*/Rgba8Unorm as ColorFormat, Swizzle};
use hal::pass::Subpass;
use hal::pso::{PipelineStage, ShaderStageFlags, VertexInputRate};
use hal::queue::Submission;
use hal::{
    buffer,
    command,
    format as f,
    image as i,
    memory as m,
    pass,
    pool,
    pso,
    window::Extent2D,
};
use hal::{DescriptorPool, Primitive, SwapchainConfig};
use hal::{Device, Instance, PhysicalDevice, Surface, Swapchain};

use std::io::Cursor;

#[cfg_attr(rustfmt, rustfmt_skip)]
const DIMS: Extent2D = Extent2D { width: 1024,height: 768 };

const ENTRY_NAME: &str = "main";

#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
struct Vertex {
    a_Pos: [f32; 2],
    a_Uv: [f32; 2],
}

#[cfg_attr(rustfmt, rustfmt_skip)]
const QUAD: [Vertex; 6] = [
    Vertex { a_Pos: [ -0.5, 0.33 ], a_Uv: [0.0, 1.0] },
    Vertex { a_Pos: [  0.5, 0.33 ], a_Uv: [1.0, 1.0] },
    Vertex { a_Pos: [  0.5,-0.33 ], a_Uv: [1.0, 0.0] },

    Vertex { a_Pos: [ -0.5, 0.33 ], a_Uv: [0.0, 1.0] },
    Vertex { a_Pos: [  0.5,-0.33 ], a_Uv: [1.0, 0.0] },
    Vertex { a_Pos: [ -0.5,-0.33 ], a_Uv: [0.0, 0.0] },
];

const COLOR_RANGE: i::SubresourceRange = i::SubresourceRange {
    aspects: f::Aspects::COLOR,
    levels: 0 .. 1,
    layers: 0 .. 1,
};

#[cfg(any(
    feature = "vulkan",
    feature = "dx11",
    feature = "dx12",
    feature = "metal",
    feature = "gl",
    feature = "wgl"
))]
fn main() {
    #[cfg(target_arch = "wasm32")]
    console_log::init_with_level(log::Level::Debug).unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::init();

    #[cfg(not(target_arch = "wasm32"))]
    let event_loop = back::glutin::event_loop::EventLoop::new();

    #[cfg(not(target_arch = "wasm32"))]
    let wb = back::glutin::window::WindowBuilder::new()
        .with_min_inner_size(winit::dpi::LogicalSize::new(1.0, 1.0))
        .with_inner_size(winit::dpi::LogicalSize::new(
            DIMS.width as _,
            DIMS.height as _,
        ))
        .with_title("quad".to_string());
    // instantiate backend
    #[cfg(not(feature = "gl"))]
    let (_window, _instance, mut adapters, mut surface) = {
        let window = wb.build(&event_loop).unwrap();
        let instance = back::Instance::create("gfx-rs quad", 1);
        let surface = instance.create_surface(&window);
        let adapters = instance.enumerate_adapters();
        (window, instance, adapters, surface)
    };
    #[cfg(feature = "gl")]
    let (mut adapters, mut surface) = {
        #[cfg(not(target_arch = "wasm32"))]
        let window = {
            let wb = wb
                .with_fullscreen(Some(event_loop.primary_monitor()));

            let cb = back::glutin::ContextBuilder::new()
                .with_gl(back::glutin::GlRequest::GlThenGles { opengl_version: (3, 0), opengles_version: (2, 0) });

            let builder =
                back::config_context(cb, ColorFormat::SELF, None)
                    .with_vsync(true);

            let windowed_context = builder
                .build_windowed(wb, &event_loop)
                .unwrap();

            let windowed_context = unsafe { windowed_context.make_current().unwrap() };

            use crate::back::glutin::platform::unix::WindowExtUnix;
            windowed_context.swap_buffers().unwrap();
            windowed_context.window().gbm_set_crtc();

            windowed_context
        };
        #[cfg(target_arch = "wasm32")]
        let window = { back::Window };

        let surface = back::Surface::from_window(window);
        let adapters = surface.enumerate_adapters();
        (adapters, surface)
    };

    for adapter in &adapters {
        println!("{:?}", adapter.info);
    }

    let mut adapter = adapters.remove(0);
    let memory_types = adapter.physical_device.memory_properties().memory_types;
    let limits = adapter.physical_device.limits();

    // Build a new device and associated command queues
    let (device, mut queue_group) = adapter
        .open_with::<_, hal::Graphics>(1, |family| surface.supports_queue_family(family))
        .unwrap();

    let mut command_pool = unsafe {
        device.create_command_pool_typed(&queue_group, pool::CommandPoolCreateFlags::empty())
    }
    .expect("Can't create command pool");

    // Setup renderpass and pipeline
    let set_layout = unsafe {
        device.create_descriptor_set_layout(
            &[
                pso::DescriptorSetLayoutBinding {
                    binding: 0,
                    ty: pso::DescriptorType::SampledImage,
                    count: 1,
                    stage_flags: ShaderStageFlags::FRAGMENT,
                    immutable_samplers: false,
                },
                pso::DescriptorSetLayoutBinding {
                    binding: 1,
                    ty: pso::DescriptorType::Sampler,
                    count: 1,
                    stage_flags: ShaderStageFlags::FRAGMENT,
                    immutable_samplers: false,
                },
            ],
            &[],
        )
    }
    .expect("Can't create descriptor set layout");

    // Descriptors
    let mut desc_pool = unsafe {
        device.create_descriptor_pool(
            1, // sets
            &[
                pso::DescriptorRangeDesc {
                    ty: pso::DescriptorType::SampledImage,
                    count: 1,
                },
                pso::DescriptorRangeDesc {
                    ty: pso::DescriptorType::Sampler,
                    count: 1,
                },
            ],
            pso::DescriptorPoolCreateFlags::empty(),
        )
    }
    .expect("Can't create descriptor pool");
    let desc_set = unsafe { desc_pool.allocate_set(&set_layout) }.unwrap();

    // Buffer allocations
    println!("Memory types: {:?}", memory_types);

    let buffer_stride = std::mem::size_of::<Vertex>() as u64;
    let buffer_len = QUAD.len() as u64 * buffer_stride;

    assert_ne!(buffer_len, 0);
    let mut vertex_buffer =
        unsafe { device.create_buffer(buffer_len, buffer::Usage::VERTEX) }.unwrap();

    let buffer_req = unsafe { device.get_buffer_requirements(&vertex_buffer) };

    let upload_type = memory_types
        .iter()
        .enumerate()
        .position(|(id, mem_type)| {
            // type_mask is a bit field where each bit represents a memory type. If the bit is set
            // to 1 it means we can use that type for our buffer. So this code finds the first
            // memory type that has a `1` (or, is allowed), and is visible to the CPU.
            buffer_req.type_mask & (1 << id) != 0
                && mem_type.properties.contains(m::Properties::CPU_VISIBLE)
        })
        .unwrap()
        .into();

    let buffer_memory = unsafe { device.allocate_memory(upload_type, buffer_req.size) }.unwrap();

    unsafe { device.bind_buffer_memory(&buffer_memory, 0, &mut vertex_buffer) }.unwrap();

    // TODO: check transitions: read/write mapping and vertex buffer read
    unsafe {
        let mut vertices = device
            .acquire_mapping_writer::<Vertex>(&buffer_memory, 0 .. buffer_req.size)
            .unwrap();
        vertices[0 .. QUAD.len()].copy_from_slice(&QUAD);
        device.release_mapping_writer(vertices).unwrap();
    }

    // Image
    let img_data = include_bytes!("data/logo.png");

    let img = image::load(Cursor::new(&img_data[..]), image::PNG)
        .unwrap()
        .to_rgba();
    let (width, height) = img.dimensions();
    let kind = i::Kind::D2(width as i::Size, height as i::Size, 1, 1);
    let row_alignment_mask = limits.optimal_buffer_copy_pitch_alignment as u32 - 1;
    let image_stride = 4usize;
    let row_pitch = (width * image_stride as u32 + row_alignment_mask) & !row_alignment_mask;
    let upload_size = (height * row_pitch) as u64;

    let mut image_upload_buffer =
        unsafe { device.create_buffer(upload_size, buffer::Usage::TRANSFER_SRC) }.unwrap();
    let image_mem_reqs = unsafe { device.get_buffer_requirements(&image_upload_buffer) };
    let image_upload_memory =
        unsafe { device.allocate_memory(upload_type, image_mem_reqs.size) }.unwrap();

    unsafe { device.bind_buffer_memory(&image_upload_memory, 0, &mut image_upload_buffer) }
        .unwrap();

    // copy image data into staging buffer
    unsafe {
        let mut data = device
            .acquire_mapping_writer::<u8>(&image_upload_memory, 0 .. image_mem_reqs.size)
            .unwrap();
        for y in 0 .. height as usize {
            let row = &(*img)
                [y * (width as usize) * image_stride .. (y + 1) * (width as usize) * image_stride];
            let dest_base = y * row_pitch as usize;
            data[dest_base .. dest_base + row.len()].copy_from_slice(row);
        }
        device.release_mapping_writer(data).unwrap();
    }

    let mut image_logo = unsafe {
        device.create_image(
            kind,
            1,
            ColorFormat::SELF,
            i::Tiling::Optimal,
            i::Usage::TRANSFER_DST | i::Usage::SAMPLED,
            i::ViewCapabilities::empty(),
        )
    }
    .unwrap();
    dbg!("1");
    let image_req = unsafe { device.get_image_requirements(&image_logo) };

    dbg!("2");
    let device_type = memory_types
        .iter()
        .enumerate()
        .position(|(id, memory_type)| {
            image_req.type_mask & (1 << id) != 0
                && memory_type.properties.contains(m::Properties::DEVICE_LOCAL)
        })
        .unwrap()
        .into();
    let image_memory = unsafe { device.allocate_memory(device_type, image_req.size) }.unwrap();
    dbg!("3");

    unsafe { device.bind_image_memory(&image_memory, 0, &mut image_logo) }.unwrap();
    let image_srv = unsafe {
        device.create_image_view(
            &image_logo,
            i::ViewKind::D2,
            ColorFormat::SELF,
            Swizzle::NO,
            COLOR_RANGE.clone(),
        )
    }
    .unwrap();
    dbg!("4");

    let sampler = unsafe {
        device.create_sampler(i::SamplerInfo::new(i::Filter::Linear, i::WrapMode::Clamp))
    }
    .expect("Can't create sampler");

    unsafe {
        device.write_descriptor_sets(vec![
            pso::DescriptorSetWrite {
                set: &desc_set,
                binding: 0,
                array_offset: 0,
                descriptors: Some(pso::Descriptor::Image(&image_srv, i::Layout::Undefined)),
            },
            pso::DescriptorSetWrite {
                set: &desc_set,
                binding: 1,
                array_offset: 0,
                descriptors: Some(pso::Descriptor::Sampler(&sampler)),
            },
        ]);
    }
    dbg!("5");

    // copy buffer to texture
    let mut copy_fence = device.create_fence(false).expect("Could not create fence");
    dbg!("5.1");
    unsafe {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<command::OneShot>();
        cmd_buffer.begin();
        dbg!("5.2");

        let image_barrier = m::Barrier::Image {
            states: (i::Access::empty(), i::Layout::Undefined)
                .. (i::Access::TRANSFER_WRITE, i::Layout::TransferDstOptimal),
            target: &image_logo,
            families: None,
            range: COLOR_RANGE.clone(),
        };

        dbg!("5.3");
        cmd_buffer.pipeline_barrier(
            PipelineStage::TOP_OF_PIPE .. PipelineStage::TRANSFER,
            m::Dependencies::empty(),
            &[image_barrier],
        );

        dbg!("5.4");
        cmd_buffer.copy_buffer_to_image(
            &image_upload_buffer,
            &image_logo,
            i::Layout::TransferDstOptimal,
            &[command::BufferImageCopy {
                buffer_offset: 0,
                buffer_width: row_pitch / (image_stride as u32),
                buffer_height: height as u32,
                image_layers: i::SubresourceLayers {
                    aspects: f::Aspects::COLOR,
                    level: 0,
                    layers: 0 .. 1,
                },
                image_offset: i::Offset { x: 0, y: 0, z: 0 },
                image_extent: i::Extent {
                    width,
                    height,
                    depth: 1,
                },
            }],
        );

        dbg!("5.5");
        let image_barrier = m::Barrier::Image {
            states: (i::Access::TRANSFER_WRITE, i::Layout::TransferDstOptimal)
                .. (i::Access::SHADER_READ, i::Layout::ShaderReadOnlyOptimal),
            target: &image_logo,
            families: None,
            range: COLOR_RANGE.clone(),
        };
        cmd_buffer.pipeline_barrier(
            PipelineStage::TRANSFER .. PipelineStage::FRAGMENT_SHADER,
            m::Dependencies::empty(),
            &[image_barrier],
        );

        dbg!("5.6");
        cmd_buffer.finish();
        dbg!("5.7");

        queue_group.queues[0].submit_without_semaphores(Some(&cmd_buffer), Some(&mut copy_fence));
        dbg!("5.8");

        device
            .wait_for_fence(&copy_fence, !0)
            .expect("Can't wait for fence");
    }
    dbg!("6");

    unsafe {
        device.destroy_fence(copy_fence);
    }

    let (caps, formats, _present_modes) = surface.compatibility(&mut adapter.physical_device);
    println!("formats: {:?}", formats);
    let format = formats.map_or(f::Format::Rgba8Srgb, |formats| {
        formats
            .iter()
            .find(|format| format.base_format().1 == ChannelType::Srgb)
            .map(|format| *format)
            .unwrap_or(formats[0])
    });

    let swap_config = SwapchainConfig::from_caps(&caps, format, DIMS);
    println!("{:?}", swap_config);
    let extent = swap_config.extent.to_extent();

    let (swap_chain, backbuffer) =
        unsafe { device.create_swapchain(&mut surface, swap_config, None) }
            .expect("Can't create swapchain");
    dbg!("7");

    let render_pass = {
        let attachment = pass::Attachment {
            format: Some(format),
            samples: 1,
            ops: pass::AttachmentOps::new(
                pass::AttachmentLoadOp::Clear,
                pass::AttachmentStoreOp::Store,
            ),
            stencil_ops: pass::AttachmentOps::DONT_CARE,
            layouts: i::Layout::Undefined .. i::Layout::Present,
        };

        let subpass = pass::SubpassDesc {
            colors: &[(0, i::Layout::ColorAttachmentOptimal)],
            depth_stencil: None,
            inputs: &[],
            resolves: &[],
            preserves: &[],
        };

        let dependency = pass::SubpassDependency {
            passes: pass::SubpassRef::External .. pass::SubpassRef::Pass(0),
            stages: PipelineStage::COLOR_ATTACHMENT_OUTPUT
                .. PipelineStage::COLOR_ATTACHMENT_OUTPUT,
            accesses: i::Access::empty()
                .. (i::Access::COLOR_ATTACHMENT_READ | i::Access::COLOR_ATTACHMENT_WRITE),
        };

        unsafe { device.create_render_pass(&[attachment], &[subpass], &[dependency]) }
            .expect("Can't create render pass")
    };
    dbg!("8");

    let backbuffer_rc = std::rc::Rc::new(std::cell::RefCell::new(backbuffer));

    let (frame_images, framebuffers) = {
        let pairs = backbuffer_rc.borrow()
            .iter()
            .map(|image| unsafe {
                let rtv = device
                    .create_image_view(
                        &image,
                        i::ViewKind::D2,
                        format,
                        Swizzle::NO,
                        COLOR_RANGE.clone(),
                    )
                    .unwrap();
                (*image, rtv)
            })
            .collect::<Vec<_>>();
        let fbos = pairs
            .iter()
            .map(|&(_, ref rtv)| unsafe {
                device
                    .create_framebuffer(&render_pass, Some(rtv), extent)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        (pairs, fbos)
    };

    // Define maximum number of frames we want to be able to be "in flight" (being computed
    // simultaneously) at once
    let frames_in_flight = 3;

    // Number of image acquisition semaphores is based on the number of swapchain images, not frames in flight,
    // plus one extra which we can guarantee is unused at any given time by swapping it out with the ones
    // in the rest of the queue.
    let mut image_acquire_semaphores = Vec::with_capacity(frame_images.len());
    let mut free_acquire_semaphore = device
        .create_semaphore()
        .expect("Could not create semaphore");

    // The number of the rest of the resources is based on the frames in flight.
    let mut submission_complete_semaphores = Vec::with_capacity(frames_in_flight);
    let mut submission_complete_fences = Vec::with_capacity(frames_in_flight);
    // Note: We don't really need a different command pool per frame in such a simple demo like this,
    // but in a more 'real' application, it's generally seen as optimal to have one command pool per
    // thread per frame. There is a flag that lets a command pool reset individual command buffers
    // which are created from it, but by default the whole pool (and therefore all buffers in it)
    // must be reset at once. Furthermore, it is often the case that resetting a whole pool is actually
    // faster and more efficient for the hardware than resetting individual command buffers, so it's
    // usually best to just make a command pool for each set of buffers which need to be reset at the
    // same time (each frame). In our case, each pool will only have one command buffer created from it,
    // though.
    let mut cmd_pools = Vec::with_capacity(frames_in_flight);
    let mut cmd_buffers = Vec::with_capacity(frames_in_flight);
    dbg!("9");

    cmd_pools.push(command_pool);
    for _ in 1 .. frames_in_flight {
        unsafe {
            cmd_pools.push(
                device
                    .create_command_pool_typed(&queue_group, pool::CommandPoolCreateFlags::empty())
                    .expect("Can't create command pool"),
            );
        }
    }

    for _ in 0 .. frame_images.len() {
        image_acquire_semaphores.push(
            device
                .create_semaphore()
                .expect("Could not create semaphore"),
        );
    }

    for i in 0 .. frames_in_flight {
        submission_complete_semaphores.push(
            device
                .create_semaphore()
                .expect("Could not create semaphore"),
        );
        submission_complete_fences.push(
            device
                .create_fence(true)
                .expect("Could not create semaphore"),
        );
        cmd_buffers.push(cmd_pools[i].acquire_command_buffer::<command::MultiShot>());
    }

    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            std::iter::once(&set_layout),
            &[(pso::ShaderStageFlags::VERTEX, 0 .. 8)],
        )
    }
    .expect("Can't create pipeline layout");
    let pipeline = {
        let vs_module = {
            let spirv =
                hal::read_spirv(Cursor::new(&include_bytes!("data/quad.vert.spv")[..])).unwrap();
            unsafe { device.create_shader_module(&spirv) }.unwrap()
        };
        let fs_module = {
            let spirv =
                hal::read_spirv(Cursor::new(&include_bytes!("./data/quad.frag.spv")[..])).unwrap();
            unsafe { device.create_shader_module(&spirv) }.unwrap()
        };

        let pipeline = {
            let (vs_entry, fs_entry) = (
                pso::EntryPoint {
                    entry: ENTRY_NAME,
                    module: &vs_module,
                    specialization: hal::spec_const_list![0.8f32],
                },
                pso::EntryPoint {
                    entry: ENTRY_NAME,
                    module: &fs_module,
                    specialization: pso::Specialization::default(),
                },
            );

            let shader_entries = pso::GraphicsShaderSet {
                vertex: vs_entry,
                hull: None,
                domain: None,
                geometry: None,
                fragment: Some(fs_entry),
            };

            let subpass = Subpass {
                index: 0,
                main_pass: &render_pass,
            };

            let mut pipeline_desc = pso::GraphicsPipelineDesc::new(
                shader_entries,
                Primitive::TriangleList,
                pso::Rasterizer::FILL,
                &pipeline_layout,
                subpass,
            );
            pipeline_desc.blender.targets.push(pso::ColorBlendDesc(
                pso::ColorMask::ALL,
                pso::BlendState::ALPHA,
            ));
            pipeline_desc.vertex_buffers.push(pso::VertexBufferDesc {
                binding: 0,
                stride: std::mem::size_of::<Vertex>() as u32,
                rate: VertexInputRate::Vertex,
            });

            pipeline_desc.attributes.push(pso::AttributeDesc {
                location: 0,
                binding: 0,
                element: pso::Element {
                    format: f::Format::Rg32Sfloat,
                    offset: 0,
                },
            });
            pipeline_desc.attributes.push(pso::AttributeDesc {
                location: 1,
                binding: 0,
                element: pso::Element {
                    format: f::Format::Rg32Sfloat,
                    offset: 8,
                },
            });

            unsafe { device.create_graphics_pipeline(&pipeline_desc, None) }
        };

        unsafe {
            device.destroy_shader_module(vs_module);
        }
        unsafe {
            device.destroy_shader_module(fs_module);
        }

        pipeline.unwrap()
    };
    dbg!("10");

    // Rendering setup
    let mut viewport = pso::Viewport {
        rect: pso::Rect {
            x: 0,
            y: 0,
            w: extent.width as _,
            h: extent.height as _,
        },
        depth: 0.0 .. 1.0,
    };

    //
    let mut recreate_swapchain = false;
    let resize_dims = Extent2D {
        width: 0,
        height: 0,
    };
    let mut frame: u64 = 0;

    let swap_chain_rc = std::cell::RefCell::new(swap_chain);
    let framebuffers_rc = std::cell::RefCell::new(framebuffers);
    let frame_images_rc = std::cell::RefCell::new(frame_images);

    dbg!("11");

    #[cfg(not(target_arch = "wasm32"))]
    event_loop.run(move |event, _, control_flow| {
        println!("{:?}", event);
        *control_flow = back::glutin::event_loop::ControlFlow::Wait;

        let swap_chain = &swap_chain_rc;
        let framebuffers = &framebuffers_rc;
        let backbuffer = &backbuffer_rc;
        let frame_images = &frame_images_rc;

        dbg!("12");

        match event {
            back::glutin::event::Event::LoopDestroyed => return,
            back::glutin::event::Event::WindowEvent { event, .. } => match event {
                back::glutin::event::WindowEvent::KeyboardInput {
                    input:
                        back::glutin::event::KeyboardInput {
                            virtual_keycode: Some(back::glutin::event::VirtualKeyCode::Escape),
                            ..
                        },
                    ..
                }
                | back::glutin::event::WindowEvent::CloseRequested => {
                    *control_flow = back::glutin::event_loop::ControlFlow::Exit
                }
                back::glutin::event::WindowEvent::Resized(dims) => {
                    println!("resized to {:?}", dims);
                    #[cfg(feature = "gl")]
                    {
                        unimplemented!();
                        // let context = surface.get_context();
                        // context.resize(dims.to_physical(context.window().get_hidpi_factor()));
                    }
                    recreate_swapchain = true;
                    resize_dims.width = dims.width as u32;
                    resize_dims.height = dims.height as u32;
                }
                _ => (),
            },
            _ => (),
        }

        dbg!("13");

        // Window was resized so we must recreate swapchain and framebuffers
        if recreate_swapchain {
            device.wait_idle().unwrap();

            dbg!("14");
            let (caps, formats, _present_modes) =
                surface.compatibility(&mut adapter.physical_device);
            // Verify that previous format still exists so we may reuse it.
            assert!(formats.iter().any(|fs| fs.contains(&format)));

            dbg!("15");
            let swap_config = SwapchainConfig::from_caps(&caps, format, resize_dims);
            println!("{:?}", swap_config);
            let extent = swap_config.extent.to_extent();

            let (new_swap_chain, new_backbuffer) =
                unsafe { device.create_swapchain(&mut surface, swap_config, None) }
                    .expect("Can't create swapchain");

            dbg!("16");
            let _ = swap_chain.replace(new_swap_chain);
            let _ = backbuffer.replace(new_backbuffer);
            // backbuffer = new_backbuffer;
            // swap_chain = new_swap_chain;

            let (new_frame_images, new_framebuffers) = {
                let backbuffer = &*backbuffer.borrow_mut();
                let pairs = backbuffer
                    .into_iter()
                    .map(|image| unsafe {
                        let rtv = device
                            .create_image_view(
                                &image,
                                i::ViewKind::D2,
                                format,
                                Swizzle::NO,
                                COLOR_RANGE.clone(),
                            )
                            .unwrap();
                        (*image, rtv)
                    })
                    .collect::<Vec<_>>();
                let fbos = pairs
                    .iter()
                    .map(|&(_, ref rtv)| unsafe {
                        device
                            .create_framebuffer(&render_pass, Some(rtv), extent)
                            .unwrap()
                    })
                    .collect();
                (pairs, fbos)
            };

            dbg!("17");

            // framebuffers = new_framebuffers;
            // frame_images = new_frame_images;
            let old_framebuffers = framebuffers.replace(new_framebuffers);
            let old_frame_images = frame_images.replace(new_frame_images);
            viewport.rect.w = extent.width as _;
            viewport.rect.h = extent.height as _;
            recreate_swapchain = false;

            dbg!("18");

            unsafe {
                // Clean up the old framebuffers, images and swapchain
                for framebuffer in old_framebuffers {
                    device.destroy_framebuffer(framebuffer);
                }
                for (_, rtv) in old_frame_images {
                    device.destroy_image_view(rtv);
                }
            }

            dbg!("19");
        }

        // Use guaranteed unused acquire semaphore to get the index of the next frame we will render to
        // by using acquire_image
        let swap_image = unsafe {
            match swap_chain.borrow_mut().acquire_image(!0, Some(&free_acquire_semaphore), None) {
                Ok((i, _)) => i as usize,
                Err(_) => {
                    recreate_swapchain = true;
                    return;
                }
            }
        };

        // Swap the acquire semaphore with the one previously associated with the image we are acquiring
        core::mem::swap(
            &mut free_acquire_semaphore,
            &mut image_acquire_semaphores[swap_image],
        );

        // Compute index into our resource ring buffers based on the frame number
        // and number of frames in flight. Pay close attention to where this index is needed
        // versus when the swapchain image index we got from acquire_image is needed.
        let frame_idx = frame as usize % frames_in_flight;

        // Wait for the fence of the previous submission of this frame and reset it; ensures we are
        // submitting only up to maximum number of frames_in_flight if we are submitting faster than
        // the gpu can keep up with. This would also guarantee that any resources which need to be
        // updated with a CPU->GPU data copy are not in use by the GPU, so we can perform those updates.
        // In this case there are none to be done, however.
        unsafe {
            device
                .wait_for_fence(&submission_complete_fences[frame_idx], !0)
                .expect("Failed to wait for fence");
            device
                .reset_fence(&submission_complete_fences[frame_idx])
                .expect("Failed to reset fence");
            cmd_pools[frame_idx].reset(false);
        }

        // Rendering
        let cmd_buffer = &mut cmd_buffers[frame_idx];
        unsafe {
            cmd_buffer.begin(false);

            cmd_buffer.set_viewports(0, &[viewport.clone()]);
            cmd_buffer.set_scissors(0, &[viewport.rect]);
            cmd_buffer.bind_graphics_pipeline(&pipeline);
            cmd_buffer.bind_vertex_buffers(0, Some((&vertex_buffer, 0)));
            cmd_buffer.bind_graphics_descriptor_sets(&pipeline_layout, 0, Some(&desc_set), &[]);

            {
                let mut encoder = cmd_buffer.begin_render_pass_inline(
                    &render_pass,
                    &framebuffers.borrow()[swap_image],
                    viewport.rect,
                    &[command::ClearValue::Color(command::ClearColor::Sfloat([
                        0.8, 0.8, 0.8, 1.0,
                    ]))],
                );
                encoder.draw(0 .. 6, 0 .. 1);
            }

            cmd_buffer.finish();

            let submission = Submission {
                command_buffers: Some(&*cmd_buffer),
                wait_semaphores: Some((
                    &image_acquire_semaphores[swap_image],
                    PipelineStage::COLOR_ATTACHMENT_OUTPUT,
                )),
                signal_semaphores: Some(&submission_complete_semaphores[frame_idx]),
            };
            queue_group.queues[0].submit(submission, Some(&submission_complete_fences[frame_idx]));

            // present frame
            if let Err(_) = swap_chain.borrow().present(
                &mut queue_group.queues[0],
                swap_image as hal::SwapImageIndex,
                Some(&submission_complete_semaphores[frame_idx]),
            ) {
                recreate_swapchain = true;
            }
        }
        // Increment our frame
        frame += 1;
    });

    // cleanup!
    device.wait_idle().unwrap();
    unsafe {
        device.destroy_descriptor_pool(desc_pool);
        device.destroy_descriptor_set_layout(set_layout);

        device.destroy_buffer(vertex_buffer);
        device.destroy_buffer(image_upload_buffer);
        device.destroy_image(image_logo);
        device.destroy_image_view(image_srv);
        device.destroy_sampler(sampler);
        device.destroy_semaphore(free_acquire_semaphore);
        for p in cmd_pools {
            device.destroy_command_pool(p.into_raw());
        }
        for s in image_acquire_semaphores {
            device.destroy_semaphore(s);
        }
        for s in submission_complete_semaphores {
            device.destroy_semaphore(s);
        }
        for f in submission_complete_fences {
            device.destroy_fence(f);
        }
        device.destroy_render_pass(render_pass);
        device.free_memory(buffer_memory);
        device.free_memory(image_memory);
        device.free_memory(image_upload_memory);
        device.destroy_graphics_pipeline(pipeline);
        device.destroy_pipeline_layout(pipeline_layout);
        for framebuffer in framebuffers {
            device.destroy_framebuffer(framebuffer);
        }
        for (_, rtv) in frame_images {
            device.destroy_image_view(rtv);
        }

        device.destroy_swapchain(swap_chain);
    }
}

#[cfg(not(any(
    feature = "vulkan",
    feature = "dx11",
    feature = "dx12",
    feature = "metal",
    feature = "gl",
    feature = "wgl"
)))]
fn main() {
    println!("You need to enable the native API feature (vulkan/metal/dx11/dx12/gl/wgl) in order to test the LL");
}
