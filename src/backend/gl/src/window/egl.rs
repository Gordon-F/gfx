//! EGL-based surface and swapchain.

use crate::{conv, native, GlContainer, PhysicalDevice, Starc};
use glow::HasContext;
use hal::{image, window as w};
use std::{os::raw, ptr};

#[derive(Debug)]
pub struct Swapchain {
    framebuffer: glow::Framebuffer,
    renderbuffer: glow::Renderbuffer,
    /// Extent because the window lies
    extent: w::Extent2D,
    format: native::TextureFormat,
    channel: hal::format::ChannelType,
}

#[derive(Debug)]
pub struct Instance {
    egl: Starc<egl::DynamicInstance>,
    display: egl::Display,
    version: (i32, i32),
    config: egl::Config,
    context: egl::Context,
    supports_native_window: bool,
    wsi_library: Option<libloading::Library>,
}

unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

type XOpenDisplayFun =
    unsafe extern "system" fn(display_name: *const raw::c_char) -> *mut raw::c_void;

impl hal::Instance<crate::Backend> for Instance {
    fn create(_: &str, _: u32) -> Result<Self, hal::UnsupportedBackend> {
        let egl = match unsafe { egl::DynamicInstance::load() } {
            Ok(egl) => Starc::new(egl),
            Err(e) => {
                log::warn!("Unable to open libEGL.so: {:?}", e);
                return Err(hal::UnsupportedBackend);
            }
        };

        let client_extensions = egl
            .query_string(None, egl::EXTENSIONS)
            .map_err(|_| hal::UnsupportedBackend)?
            .to_string_lossy();
        log::info!("Client extensions: {:?}", client_extensions);
        let client_ext_list = client_extensions.split_whitespace().collect::<Vec<_>>();

        let mut wsi_library = None;
        let x11_display = if client_ext_list.contains(&"EGL_EXT_platform_x11") {
            log::info!("Loading X11 library to get the current display");
            if let Ok(library) = libloading::Library::new("libX11.so") {
                let func: libloading::Symbol<XOpenDisplayFun> =
                    unsafe { library.get(b"XOpenDisplay").unwrap() };
                let result = unsafe { func(ptr::null()) };
                wsi_library = Some(library);
                ptr::NonNull::new(result)
            } else {
                None
            }
        } else {
            None
        };
        let display = if let Some(x11_display) = x11_display {
            log::info!("Using X11 platform");
            const EGL_PLATFORM_X11_KHR: u32 = 0x31D5;
            let display_attributes = [egl::ATTRIB_NONE];
            egl.get_platform_display(
                EGL_PLATFORM_X11_KHR,
                x11_display.as_ptr(),
                &display_attributes,
            )
            .unwrap()
        } else {
            log::info!("Using default platform");
            egl.get_display(egl::DEFAULT_DISPLAY).unwrap()
        };

        let version = egl
            .initialize(display)
            .map_err(|_| hal::UnsupportedBackend)?;
        let vendor = egl.query_string(Some(display), egl::VENDOR).unwrap();
        let display_extensions = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .unwrap()
            .to_string_lossy();
        log::info!(
            "Display vendor {:?}, version {:?}, extensions: {:?}",
            vendor,
            version,
            display_extensions
        );
        if version < (1, 4) {
            log::error!("EGL supported version is only {:?}", version);
            return Err(hal::UnsupportedBackend);
        }
        let display_ext_list = display_extensions.split_whitespace().collect::<Vec<_>>();
        let required_display_extensions = ["EGL_KHR_create_context", "EGL_KHR_surfaceless_context"];
        for required_ext in required_display_extensions.iter() {
            if !display_ext_list.contains(required_ext) {
                log::warn!("{} is not present", required_ext);
            }
        }

        if log::max_level() >= log::LevelFilter::Trace {
            log::trace!("Configurations:");
            let config_count = egl.get_config_count(display).unwrap();
            let mut configurations = Vec::with_capacity(config_count);
            egl.get_configs(display, &mut configurations).unwrap();
            for &config in configurations.iter() {
                log::trace!("\tCONFORMANT=0x{:X}, RENDERABLE=0x{:X}, NATIVE_RENDERABLE=0x{:X}, SURFACE_TYPE=0x{:X}",
                    egl.get_config_attrib(display, config, egl::CONFORMANT).unwrap(),
                    egl.get_config_attrib(display, config, egl::RENDERABLE_TYPE).unwrap(),
                    egl.get_config_attrib(display, config, egl::NATIVE_RENDERABLE).unwrap(),
                    egl.get_config_attrib(display, config, egl::SURFACE_TYPE).unwrap(),
                );
            }
        }

        //Note: only GLES is supported here.
        let mut supports_native_window = true;
        //TODO: EGL_SLOW_CONFIG
        let config_attributes = [
            egl::CONFORMANT,
            egl::OPENGL_ES2_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::NATIVE_RENDERABLE,
            egl::TRUE as _,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::NONE,
        ];
        let pre_config = match wsi_library {
            Some(_) => egl.choose_first_config(display, &config_attributes),
            None => Ok(None),
        };
        let config = match pre_config {
            Ok(Some(config)) => config,
            Ok(None) => {
                log::warn!("no compatible EGL config found, trying off-screen");
                // Android surface still presentable
                supports_native_window = if cfg!(target_os = "android") {
                    true
                } else {
                    false
                };
                let reduced_config_attributes =
                    [egl::RENDERABLE_TYPE, egl::OPENGL_ES2_BIT, egl::NONE];
                match egl.choose_first_config(display, &reduced_config_attributes) {
                    Ok(Some(config)) => config,
                    _ => return Err(hal::UnsupportedBackend),
                }
            }
            Err(e) => {
                log::error!("error in choose_first_config: {:?}", e);
                return Err(hal::UnsupportedBackend);
            }
        };

        egl.bind_api(egl::OPENGL_ES_API).unwrap();

        //TODO: make it so `Device` == EGL Context
        let mut context_attributes = vec![
            egl::CONTEXT_CLIENT_VERSION,
            3, // Request GLES 3.0 or higher
        ];
        if cfg!(debug_assertions) && wsi_library.is_none() && cfg!(not(target_os = "android")) {
            //TODO: figure out why this is needed
            context_attributes.push(egl::CONTEXT_OPENGL_DEBUG);
            context_attributes.push(egl::TRUE as _);
        }
        context_attributes.push(egl::NONE as _);
        let context = match egl.create_context(display, config, None, &context_attributes) {
            Ok(context) => context,
            Err(e) => {
                log::warn!("unable to create GLES 3.x context: {:?}", e);
                return Err(hal::UnsupportedBackend);
            }
        };

        Ok(Instance {
            egl,
            display,
            version,
            config,
            context,
            supports_native_window,
            wsi_library,
        })
    }

    fn enumerate_adapters(&self) -> Vec<hal::adapter::Adapter<crate::Backend>> {
        self.egl
            .make_current(self.display, None, None, Some(self.context))
            .unwrap();
        let context = unsafe {
            glow::Context::from_loader_function(|name| {
                match self.egl.get_proc_address(name) {
                    Some(ext) => ext as *const _,
                    _ => std::ptr::null() as *const _,
                }
            })
        };
        // Create physical device
        vec![PhysicalDevice::new_adapter(context)]
    }

    unsafe fn create_surface(
        &self,
        has_handle: &impl raw_window_handle::HasRawWindowHandle,
    ) -> Result<Surface, w::InitError> {
        use raw_window_handle::RawWindowHandle as Rwh;
        let mut native_window = match has_handle.raw_window_handle() {
            #[cfg(not(target_os = "android"))]
            Rwh::Xlib(handle) => handle.window,
            #[cfg(not(target_os = "android"))]
            Rwh::Xcb(handle) => handle.window as _,
            #[cfg(target_os = "android")]
            Rwh::Android(handle) => handle.a_native_window,
            other => panic!("Unsupported window: {:?}", other),
        };
        let attributes = [
            egl::RENDER_BUFFER as usize,
            egl::BACK_BUFFER as usize,
            // Always enable sRGB
            egl::GL_COLORSPACE as usize,
            egl::GL_COLORSPACE_SRGB as usize,
            egl::ATTRIB_NONE,
        ];

        let native_window_ptr = match has_handle.raw_window_handle() {
            #[cfg(target_os = "android")]
            Rwh::Android(_) => native_window as *mut _ as _,
            _ => &mut native_window as *mut _ as *mut _,
        };

        match self.egl.create_platform_window_surface(
            self.display,
            self.config,
            native_window_ptr,
            &attributes,
        ) {
            Ok(raw) => Ok(Surface {
                egl: self.egl.clone(),
                raw,
                display: self.display,
                context: self.context,
                presentable: self.supports_native_window,
                swapchain: None,
            }),
            Err(e) => {
                log::warn!("Error in create_window_surface: {:?}", e);
                Err(w::InitError::UnsupportedWindowHandle)
            }
        }
    }

    unsafe fn destroy_surface(&self, surface: Surface) {
        self.egl.destroy_surface(self.display, surface.raw).unwrap();
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Err(e) = self.egl.destroy_context(self.display, self.context) {
            log::warn!("Error in destroy_context: {:?}", e);
        }
        if let Err(e) = self.egl.terminate(self.display) {
            log::warn!("Error in terminate: {:?}", e);
        }
    }
}

#[derive(Debug)]
pub struct Surface {
    egl: Starc<egl::DynamicInstance>,
    raw: egl::Surface,
    display: egl::Display,
    context: egl::Context,
    presentable: bool,
    pub(crate) swapchain: Option<Swapchain>,
}

unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

impl w::PresentationSurface<crate::Backend> for Surface {
    type SwapchainImage = native::SwapchainImage;

    unsafe fn configure_swapchain(
        &mut self,
        device: &crate::Device,
        config: w::SwapchainConfig,
    ) -> Result<(), w::SwapchainError> {
        self.unconfigure_swapchain(device);

        let desc = conv::describe_format(config.format).unwrap();

        let gl = &device.share.context;
        let renderbuffer = gl.create_renderbuffer().unwrap();
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(renderbuffer));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            desc.tex_internal,
            config.extent.width as _,
            config.extent.height as _,
        );
        let framebuffer = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_renderbuffer(
            glow::READ_FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(renderbuffer),
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);

        self.swapchain = Some(Swapchain {
            renderbuffer,
            framebuffer,
            extent: config.extent,
            format: desc.tex_internal,
            channel: config.format.base_format().1,
        });

        Ok(())
    }

    unsafe fn unconfigure_swapchain(&mut self, device: &crate::Device) {
        let gl = &device.share.context;
        if let Some(sc) = self.swapchain.take() {
            gl.delete_renderbuffer(sc.renderbuffer);
            gl.delete_framebuffer(sc.framebuffer);
        }
    }

    unsafe fn acquire_image(
        &mut self,
        _timeout_ns: u64,
    ) -> Result<(Self::SwapchainImage, Option<w::Suboptimal>), w::AcquireError> {
        let sc = self.swapchain.as_ref().unwrap();
        let sc_image =
            native::SwapchainImage::new(sc.renderbuffer, sc.format, sc.extent, sc.channel);
        Ok((sc_image, None))
    }
}

impl w::Surface<crate::Backend> for Surface {
    fn supports_queue_family(&self, _: &crate::QueueFamily) -> bool {
        self.presentable
    }

    fn capabilities(&self, _physical_device: &PhysicalDevice) -> w::SurfaceCapabilities {
        w::SurfaceCapabilities {
            present_modes: w::PresentMode::FIFO,                  //TODO
            composite_alpha_modes: w::CompositeAlphaMode::OPAQUE, //TODO
            image_count: 2..=2,
            current_extent: None,
            extents: w::Extent2D {
                width: 4,
                height: 4,
            }..=w::Extent2D {
                width: 4096,
                height: 4096,
            },
            max_image_layers: 1,
            usage: image::Usage::COLOR_ATTACHMENT,
        }
    }

    fn supported_formats(
        &self,
        _physical_device: &PhysicalDevice,
    ) -> Option<Vec<hal::format::Format>> {
        use hal::format::Format;
        Some(vec![Format::Rgba8Srgb, Format::Bgra8Srgb])
    }
}

impl Surface {
    pub(crate) unsafe fn present(
        &mut self,
        _image: native::SwapchainImage,
        gl: &GlContainer,
    ) -> Result<Option<w::Suboptimal>, w::PresentError> {
        let sc = self.swapchain.as_ref().unwrap();

        self.egl
            .make_current(
                self.display,
                Some(self.raw),
                Some(self.raw),
                Some(self.context),
            )
            .unwrap();
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(sc.framebuffer));
        gl.blit_framebuffer(
            0,
            0,
            sc.extent.width as _,
            sc.extent.height as _,
            0,
            0,
            sc.extent.width as _,
            sc.extent.height as _,
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
        );
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);

        self.egl.swap_buffers(self.display, self.raw).unwrap();
        self.egl
            .make_current(self.display, None, None, Some(self.context))
            .unwrap();

        Ok(None)
    }
}
