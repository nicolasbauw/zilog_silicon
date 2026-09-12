//! Contexte wgpu+egui partagé par les fenêtres "egui seul" — celles qui
//! n'affichent que des panneaux egui, sans rien du pipeline de rendu CPC
//! (`renderer.rs`, qui a en plus le pipeline de rendu CPC, ne s'en sert
//! pas). Utilisé par la fenêtre de statut (F12, `status_panel.rs`) et la
//! console complète (F11, `console_window.rs`) : troisième copie quasi
//! identique du même code d'initialisation évitée en le factorisant ici.
//!
//! Contexte GPU volontairement indépendant de celui de la fenêtre
//! principale : ces fenêtres secondaires, cachées par défaut et peu
//! sollicitées, ne justifient pas le risque de coupler leur cycle de vie à
//! celui du rendu de la trame émulée. Le coût (un contexte wgpu de plus par
//! fenêtre) est négligeable au regard de ce que ça évite de complexité.

use egui_sdl2_event::EguiSDL2State;
use sdl2::video::Window;

pub struct EguiGpu {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub egui_ctx: egui::Context,
    pub egui_state: EguiSDL2State,
    pub egui_renderer: egui_wgpu::Renderer,
}

impl EguiGpu {
    /// # Sécurité
    ///
    /// La `wgpu::Surface` retournée n'a aucun lien de durée de vie formel
    /// avec `window` (voir `renderer::Renderer`, même contrainte) :
    /// l'appelant doit garantir que `window` reste en vie au moins aussi
    /// longtemps qu'elle — en la possédant, typiquement, et en la
    /// déclarant après le champ qui contient cette valeur (Rust détruit les
    /// champs d'une struct dans leur ordre de déclaration).
    pub fn new(window: &Window, device_label: &str) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // SAFETY : voir le commentaire de sécurité sur `EguiGpu::new`.
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(window).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| e.to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(device_label),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| e.to_string())?;

        let (draw_w, draw_h) = window.drawable_size();
        let surface_caps = surface.get_capabilities(&adapter);
        // Non-sRGB, comme le recommande la doc d'`egui_wgpu::Renderer::new` :
        // le shader d'egui écrit déjà des valeurs en espace gamma, un format
        // de surface *_Srgb lui appliquerait un second gamma au moment du
        // blending.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: draw_w.max(1),
            height: draw_h.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let egui_ctx = egui::Context::default();
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, surface_format, egui_wgpu::RendererOptions::default());
        // dpi_scaling à 1.0 : `drawable_size` donne déjà des pixels physiques,
        // et ces fenêtres n'ont pas besoin d'être nettes sur un écran HiDPI à
        // tout prix pour de simples panneaux de texte.
        let egui_state = EguiSDL2State::new(draw_w, draw_h, 1.0);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            egui_ctx,
            egui_state,
            egui_renderer,
        })
    }

    /// À appeler pour chaque événement SDL2 reçu, qu'il concerne cette
    /// fenêtre ou non : `EguiSDL2State` filtre lui-même sur l'identifiant de
    /// fenêtre.
    pub fn handle_event(&mut self, window: &Window, event: &sdl2::event::Event) {
        self.egui_state.sdl2_input_to_egui(window, event);
    }

    /// À appeler sur tout `WindowEvent::SizeChanged`/`Resized` de cette
    /// fenêtre.
    pub fn resize(&mut self, window: &Window) {
        let (w, h) = window.drawable_size();
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.device, &self.config);
        self.egui_state.update_screen_rect(w, h);
    }

    /// Point commun à `StatusPanel::render` et `ConsoleWindow::render` :
    /// construit la trame egui (`build_ui`), l'envoie à l'écran, et
    /// applique la sortie egui (curseur, presse-papiers) sur `window`.
    /// `start` est l'horloge propre à l'appelant (`egui_state.raw_input.time`
    /// veut un temps monotone depuis un instant de référence quelconque).
    pub fn present(
        &mut self,
        window: &Window,
        start: std::time::Instant,
        build_ui: impl FnMut(&egui::Context),
    ) {
        self.egui_state
            .update_time(Some(start.elapsed().as_secs_f64()), 1.0 / 60.0);
        let raw_input = std::mem::take(&mut self.egui_state.raw_input);
        let full_output = self.egui_ctx.run(raw_input, build_ui);
        self.egui_state
            .process_output(window, &full_output.platform_output);

        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            Err(_) => {
                self.resize(window);
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let paint_jobs = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui window encoder"),
            });
        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui window pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // `egui_wgpu::Renderer::render` exige une passe 'static : elle
            // garde en vie elle-même toutes les ressources dont elle a
            // besoin, voir sa documentation.
            let mut rpass = rpass.forget_lifetime();
            self.egui_renderer
                .render(&mut rpass, &paint_jobs, &screen_descriptor);
        }
        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        self.queue.submit([encoder.finish()]);
        output.present();
    }
}
