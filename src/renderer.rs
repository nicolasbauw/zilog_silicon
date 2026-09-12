//! Pipeline de rendu wgpu de la fenêtre principale (Plan V2.md, jalon M0).
//!
//! Remplace l'ancien `Canvas`/texture streaming logiciel de SDL2 : la
//! sortie de `video::render` (un buffer RGB24 `SCREEN_WIDTH`x`SCREEN_HEIGHT`,
//! inchangée) est uploadée dans une texture, puis dessinée par un quad
//! plein écran dans un viewport calculé à la main pour reproduire le
//! letterboxing/pillarboxing que `Canvas::set_logical_size` offrait
//! gratuitement — ce mécanisme n'a pas d'équivalent côté wgpu.
//!
//! Suit l'exemple officiel `raw-window-handle-with-wgpu` de la crate sdl2 :
//! seule combinaison balisée pour faire cohabiter SDL2 (fenêtrage, entrées)
//! et wgpu (rendu) dans un même process.
//!
//! Porte aussi, depuis le jalon M2, la surcouche egui de la fenêtre
//! principale (console F11, futurs panneaux F6/F7) : contrairement à la
//! fenêtre de statut (`status_panel.rs`, M1), c'est la MÊME fenêtre/surface
//! que le rendu CPC, donc le même contexte wgpu — pas de second GPU à créer,
//! juste une seconde passe de rendu par-dessus la première (`present`, en
//! `LoadOp::Load` plutôt que `Clear`, pour ne pas effacer l'image CPC déjà
//! dessinée).
//!
//! Depuis le jalon M4, deux pipelines dessinent le quad CPC : `pipeline`
//! (pixel net, inchangé) et `crt_pipeline` (scanlines + aperture arrondie
//! des pixels, `renderer_crt.wgsl`), basculés par F5. Les deux partagent le
//! même groupe de liaison 0 (texture + échantillonneur) ; seul le second a
//! besoin d'un groupe 1 (taille de l'image source, en uniforme).

use egui_sdl2_event::EguiSDL2State;
use sdl2::video::Window;
use wgpu::util::DeviceExt;

use bytebox_core::config::CrtConfig;
use bytebox_core::video;

/// Paramètres du shader CRT (`renderer_crt.wgsl`), en mémoire tampon
/// uniforme GPU : `source_size` (taille de l'image source, en pixels
/// source — jamais en pixels de sortie, voir le commentaire du shader) ne
/// change jamais après construction (`SCREEN_WIDTH`/`HEIGHT` sont fixes),
/// mais les six derniers champs reflètent `CrtSettings` et sont réécrits à
/// chaque changement depuis le panneau F6 (`set_crt_settings`).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CrtParams {
    source_size: [f32; 2],
    /// `video::PIXELS_PER_SCANLINE` : voir le commentaire du champ homonyme
    /// côté WGSL. Constant lui aussi, comme `source_size`.
    line_height: f32,
    mask_cell_px: f32,
    mask_min: f32,
    mask_strength: f32,
    scanline_beam: f32,
    scanline_strength: f32,
    beam_bloom: f32,
    bright_boost: f32,
    horizontal_blur: f32,
    /// Un uniforme WGSL est aligné sur 16 octets : 11 `f32` (44) sont
    /// complétés à 48. Doit rester en phase avec le `_padding` du shader.
    _padding: f32,
}

impl CrtParams {
    /// Complète les réglages réglables (`CrtSettings`) par ce que le shader
    /// doit savoir de la géométrie du tampon source, invariant de l'exécution.
    fn new(settings: CrtSettings) -> Self {
        Self {
            source_size: [video::SCREEN_WIDTH as f32, video::SCREEN_HEIGHT as f32],
            line_height: video::PIXELS_PER_SCANLINE as f32,
            mask_cell_px: settings.mask_cell_px,
            mask_min: settings.mask_min,
            mask_strength: settings.mask_strength,
            scanline_beam: settings.scanline_beam,
            scanline_strength: settings.scanline_strength,
            beam_bloom: settings.beam_bloom,
            bright_boost: settings.bright_boost,
            horizontal_blur: settings.horizontal_blur,
            _padding: 0.0,
        }
    }
}

/// Réglages ajustables du shader CRT (F5), exposés au panneau de
/// configuration (F6, section "Shader CRT") — voir les commentaires de
/// `renderer_crt.wgsl` pour ce que chaque champ contrôle visuellement.
/// Réglage de session par défaut (repart des valeurs par défaut à chaque
/// lancement), sauf enregistrement explicite via le bouton "Save" du
/// panneau (`CrtConfig`, section `[crt]` de `config.toml`). L'activation du
/// shader elle-même (F5) suit la même logique, mais via
/// `CrtConfig::enabled_at_startup` plutôt qu'un champ de cette struct —
/// elle ne fait pas partie de l'uniforme GPU que ces champs alimentent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CrtSettings {
    pub mask_cell_px: f32,
    pub mask_min: f32,
    pub mask_strength: f32,
    pub scanline_beam: f32,
    pub scanline_strength: f32,
    pub beam_bloom: f32,
    pub bright_boost: f32,
    pub horizontal_blur: f32,
}

impl CrtSettings {
    /// Applique les valeurs enregistrées dans `config.toml` par-dessus les
    /// valeurs par défaut, champ par champ : une section `[crt]` partielle
    /// (ou absente) reste donc parfaitement valable.
    pub fn from_config(crt: &CrtConfig) -> Self {
        let d = Self::default();
        Self {
            mask_cell_px: crt.mask_cell_px.unwrap_or(d.mask_cell_px),
            mask_min: crt.mask_min.unwrap_or(d.mask_min),
            mask_strength: crt.mask_strength.unwrap_or(d.mask_strength),
            scanline_beam: crt.scanline_beam.unwrap_or(d.scanline_beam),
            scanline_strength: crt.scanline_strength.unwrap_or(d.scanline_strength),
            beam_bloom: crt.beam_bloom.unwrap_or(d.beam_bloom),
            bright_boost: crt.bright_boost.unwrap_or(d.bright_boost),
            horizontal_blur: crt.horizontal_blur.unwrap_or(d.horizontal_blur),
        }
    }

    /// Réciproque de [`CrtSettings::from_config`], pour l'enregistrement :
    /// tous les champs sont renseignés, même ceux restés à leur valeur par
    /// défaut. Enregistrer, c'est figer un rendu — si une version ultérieure
    /// change les valeurs par défaut, l'utilisateur doit retrouver le sien.
    pub fn to_config(self) -> CrtConfig {
        CrtConfig {
            mask_cell_px: Some(self.mask_cell_px),
            mask_min: Some(self.mask_min),
            mask_strength: Some(self.mask_strength),
            scanline_beam: Some(self.scanline_beam),
            scanline_strength: Some(self.scanline_strength),
            beam_bloom: Some(self.beam_bloom),
            bright_boost: Some(self.bright_boost),
            horizontal_blur: Some(self.horizontal_blur),
            // Hors du champ de `CrtSettings` (voir sa doc) : laissé à la
            // charge de l'appelant (`config_panel.rs`), qui le renseigne
            // depuis la case "Enable at startup" avant d'enregistrer.
            enabled_at_startup: None,
        }
    }
}

impl Default for CrtSettings {
    /// Valeurs choisies par itération visuelle (Plan V2.md, jalon M4),
    /// réglées en plein écran sur un écran haute densité (4K), où l'unité
    /// "pixel de sortie" est physiquement minuscule.
    ///
    /// Un `config.toml` qui porte une section `[crt]` les outrepasse, champ
    /// par champ — voir [`CrtSettings::from_config`].
    fn default() -> Self {
        Self {
            mask_cell_px: 2.0,
            mask_min: 0.6,
            mask_strength: 0.35,
            scanline_beam: 9.0,
            scanline_strength: 0.6,
            beam_bloom: 0.66,
            bright_boost: 1.6,
            horizontal_blur: 0.65,
        }
    }
}

/// # Sécurité
///
/// `surface` est créée via `create_surface_unsafe`, qui ne lie sa durée de
/// vie à rien : c'est ce `Renderer` qui doit garantir que `window` reste en
/// vie au moins aussi longtemps qu'elle. Comme Rust détruit les champs d'une
/// struct dans leur ordre de déclaration, `surface` est déclarée avant
/// `window` pour être abandonnée en premier.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    /// Pipeline shader CRT (F5, `renderer_crt.wgsl`) : partage `bind_group`
    /// ci-dessus (groupe 0, même disposition), ajoute `crt_params_bind_group`
    /// en groupe 1.
    crt_pipeline: wgpu::RenderPipeline,
    crt_params_buffer: wgpu::Buffer,
    crt_params_bind_group: wgpu::BindGroup,
    crt_enabled: bool,
    crt_settings: CrtSettings,
    frame_texture: wgpu::Texture,
    /// Buffer de conversion RGB24 (produit par `video::render`) vers
    /// RGBA8 (seul format que wgpu accepte en texture couleur usuelle) :
    /// alloué une fois, réutilisé à chaque trame.
    rgba: Vec<u8>,
    egui_ctx: egui::Context,
    egui_state: EguiSDL2State,
    egui_renderer: egui_wgpu::Renderer,
    egui_start: std::time::Instant,
    window: Window,
}

impl Renderer {
    pub fn new(window: Window) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // SAFETY : voir le commentaire de sécurité sur `Renderer` — `window`
        // est déplacée dans cette même struct juste après, et vit donc au
        // moins aussi longtemps que `surface`.
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(&window).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| e.to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bytebox device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| e.to_string())?;

        let (draw_w, draw_h) = window.drawable_size();
        let surface_caps = surface.get_capabilities(&adapter);
        // Pas de conversion sRGB : `video::render` produit déjà les valeurs
        // finales de la palette matérielle, et l'ancien pipeline SDL2 les
        // recopiait telles quelles. Choisir un format *_Srgb ici ferait
        // appliquer un gamma supplémentaire à l'écriture, et l'image ne
        // serait plus identique à celle d'aujourd'hui.
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

        let frame_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame texture"),
            size: wgpu::Extent3d {
                width: video::SCREEN_WIDTH as u32,
                height: video::SCREEN_HEIGHT as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Pas sRGB non plus, pour la même raison que le format de
            // surface ci-dessus.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let frame_view = frame_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Nearest, pas Linear : c'est le filtrage par défaut de SDL2 (aucun
        // hint `SDL_HINT_RENDER_SCALE_QUALITY` n'était positionné), pixel
        // net à l'agrandissement. L'adoucissement de l'image agrandie est le
        // rôle du shader CRT (F5, jalon M4, `renderer_crt.wgsl`), pas de
        // cette mise à l'échelle — les deux pipelines partagent ce même
        // échantillonneur, seul le fragment shader diffère.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("frame sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frame shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("renderer_frame.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frame pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("frame pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Pipeline shader CRT (F5, jalon M4) : même groupe de liaison 0 que
        // le pipeline net ci-dessus (texture + échantillonneur, disposition
        // identique — `bind_group` est donc réutilisé tel quel), plus un
        // groupe 1 pour la taille de l'image source.
        let crt_settings = CrtSettings::default();
        let crt_params = CrtParams::new(crt_settings);
        // COPY_DST : contrairement à `source_size`, ces réglages sont
        // réécrits en direct depuis le panneau F6 (`set_crt_settings`), pas
        // seulement lus une fois à la construction.
        let crt_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crt params buffer"),
            contents: bytemuck::bytes_of(&crt_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let crt_params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crt params bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let crt_params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crt params bind group"),
            layout: &crt_params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: crt_params_buffer.as_entire_binding(),
            }],
        });
        let crt_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crt shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("renderer_crt.wgsl").into()),
        });
        let crt_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crt pipeline layout"),
            bind_group_layouts: &[&bind_group_layout, &crt_params_layout],
            push_constant_ranges: &[],
        });
        let crt_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt pipeline"),
            layout: Some(&crt_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &crt_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &crt_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Même raisonnement de format que la texture de trame CPC ci-dessus :
        // non-sRGB, comme le recommande la doc d'`egui_wgpu::Renderer::new`
        // (son shader écrit déjà des valeurs en espace gamma).
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, surface_format, egui_wgpu::RendererOptions::default());
        let egui_state = EguiSDL2State::new(draw_w, draw_h, 1.0);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_group,
            crt_pipeline,
            crt_params_buffer,
            crt_params_bind_group,
            crt_enabled: false,
            crt_settings,
            frame_texture,
            rgba: vec![0u8; video::SCREEN_WIDTH * video::SCREEN_HEIGHT * 4],
            egui_ctx: egui::Context::default(),
            egui_state,
            egui_renderer,
            egui_start: std::time::Instant::now(),
            window,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn window_mut(&mut self) -> &mut Window {
        &mut self.window
    }

    /// À appeler sur tout `WindowEvent::SizeChanged`/`Resized` de la fenêtre
    /// principale : reconfigure la surface à la nouvelle taille de rendu
    /// (en pixels physiques, pas en points — importe sur les écrans HiDPI).
    pub fn resize(&mut self) {
        let (w, h) = self.window.drawable_size();
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.device, &self.config);
        self.egui_state.update_screen_rect(w, h);
    }

    /// À appeler pour chaque événement SDL2 reçu, qu'il concerne cette
    /// fenêtre ou non : `EguiSDL2State` filtre lui-même sur l'identifiant de
    /// fenêtre (voir `status_panel.rs`, même mécanisme).
    pub fn handle_event(&mut self, event: &sdl2::event::Event) {
        self.egui_state.sdl2_input_to_egui(&self.window, event);
    }

    /// Bascule le shader CRT (F5). Repli sur le rendu net (`self.pipeline`)
    /// quand désactivé : comportement de départ inchangé, comme pour tout
    /// bascule F1-F12 de ce fichier.
    pub fn toggle_crt(&mut self) {
        self.crt_enabled = !self.crt_enabled;
    }

    /// Applique l'état initial du shader lu depuis `config.toml`
    /// (`CrtConfig::enabled_at_startup`), une fois au lancement — voir
    /// `sdl::run`. Distinct de `toggle_crt` : ici on impose un état plutôt
    /// que d'inverser le courant.
    pub fn set_crt_enabled(&mut self, enabled: bool) {
        self.crt_enabled = enabled;
    }

    /// État courant du shader CRT — utile à `sdl.rs` juste après `toggle_crt`
    /// pour savoir quel message afficher (console et OSD), sans dupliquer la
    /// bascule elle-même.
    pub fn crt_enabled(&self) -> bool {
        self.crt_enabled
    }

    pub fn crt_settings(&self) -> CrtSettings {
        self.crt_settings
    }

    /// Réécrit les réglages du shader CRT, immédiatement effectifs (le
    /// panneau F6 appelle ceci à chaque trame où il est ouvert). Coût
    /// négligeable : `write_buffer` sur 32 octets, comme la texture de trame
    /// CPC elle-même à chaque `present`.
    pub fn set_crt_settings(&mut self, settings: CrtSettings) {
        self.crt_settings = settings;
        let params = CrtParams::new(settings);
        self.queue
            .write_buffer(&self.crt_params_buffer, 0, bytemuck::bytes_of(&params));
    }

    /// Calcule le rectangle (en pixels physiques) où dessiner l'image
    /// `SCREEN_WIDTH`x`SCREEN_HEIGHT` à l'échelle maximale qui tient dans la
    /// surface sans déformer son ratio d'aspect — l'équivalent manuel de ce
    /// que faisait `Canvas::set_logical_size`.
    fn letterboxed_viewport(&self) -> (f32, f32, f32, f32) {
        letterboxed_viewport(
            self.config.width as f32,
            self.config.height as f32,
            video::SCREEN_WIDTH as f32,
            video::SCREEN_HEIGHT as f32,
        )
    }

    /// Envoie une trame (buffer RGB24 de `video::render`) à l'écran.
    ///
    /// `overlay`, s'il est fourni, construit une interface egui (console
    /// F11, futurs panneaux) dessinée par-dessus l'image CPC dans la même
    /// passe de commandes — `None` reproduit exactement le comportement du
    /// jalon M0 (aucune passe egui, aucun coût).
    pub fn present(&mut self, frame_buffer: &[u8], overlay: Option<&mut dyn FnMut(&egui::Context)>) {
        debug_assert_eq!(frame_buffer.len(), video::SCREEN_WIDTH * video::SCREEN_HEIGHT * 3);
        for (rgba, rgb) in self.rgba.chunks_exact_mut(4).zip(frame_buffer.chunks_exact(3)) {
            rgba[0] = rgb[0];
            rgba[1] = rgb[1];
            rgba[2] = rgb[2];
            rgba[3] = 255;
        }

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * video::SCREEN_WIDTH as u32),
                rows_per_image: Some(video::SCREEN_HEIGHT as u32),
            },
            wgpu::Extent3d {
                width: video::SCREEN_WIDTH as u32,
                height: video::SCREEN_HEIGHT as u32,
                depth_or_array_layers: 1,
            },
        );

        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            // Surface périmée (redimensionnement en cours, minimisation...) :
            // on retente à la trame suivante plutôt que de planter.
            Err(_) => {
                self.resize();
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    // Sans effet pour une texture 2D classique : ne s'applique
                    // qu'aux textures 3D, absentes ici.
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Noir, comme la couleur de tracé par défaut du
                        // Canvas SDL2 d'origine : c'est ce qui recouvre les
                        // bandes de letterboxing/pillarboxing.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let (x, y, w, h) = self.letterboxed_viewport();
            if w > 0.0 && h > 0.0 {
                rpass.set_viewport(x, y, w, h, 0.0, 1.0);
                rpass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
                rpass.set_bind_group(0, &self.bind_group, &[]);
                if self.crt_enabled {
                    rpass.set_pipeline(&self.crt_pipeline);
                    rpass.set_bind_group(1, &self.crt_params_bind_group, &[]);
                } else {
                    rpass.set_pipeline(&self.pipeline);
                }
                rpass.draw(0..4, 0..1);
            }
        }

        if let Some(build_ui) = overlay {
            self.egui_state.update_time(
                Some(self.egui_start.elapsed().as_secs_f64()),
                1.0 / 60.0,
            );
            let raw_input = std::mem::take(&mut self.egui_state.raw_input);
            let full_output = self.egui_ctx.run(raw_input, build_ui);
            self.egui_state
                .process_output(&self.window, &full_output.platform_output);

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
            self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &paint_jobs,
                &screen_descriptor,
            );
            {
                let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui overlay pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // Garde l'image CPC tout juste dessinée : cette
                            // passe s'ajoute par-dessus, elle ne remplace rien.
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let mut rpass = rpass.forget_lifetime();
                self.egui_renderer
                    .render(&mut rpass, &paint_jobs, &screen_descriptor);
            }
            for id in &full_output.textures_delta.free {
                self.egui_renderer.free_texture(id);
            }
        }

        self.queue.submit([encoder.finish()]);
        output.present();
    }
}

/// Rectangle `(x, y, largeur, hauteur)`, dans la surface `surf_w`x`surf_h`,
/// où dessiner une image `img_w`x`img_h` à l'échelle maximale qui y tient
/// sans déformer son ratio d'aspect — centré, le reste en bandes noires.
/// Fonction libre (plutôt que méthode sur `Renderer`) pour rester testable
/// sans device ni fenêtre.
fn letterboxed_viewport(surf_w: f32, surf_h: f32, img_w: f32, img_h: f32) -> (f32, f32, f32, f32) {
    let scale = (surf_w / img_w).min(surf_h / img_h);
    let vp_w = img_w * scale;
    let vp_h = img_h * scale;
    ((surf_w - vp_w) / 2.0, (surf_h - vp_h) / 2.0, vp_w, vp_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// x1/x2/x3 : la fenêtre est un multiple exact de l'image (800x600),
    /// donc pas de bandes du tout — comme aujourd'hui avec
    /// `Canvas::set_logical_size`.
    #[test]
    fn exact_multiples_fill_the_surface_without_bars() {
        for factor in [1.0, 2.0, 3.0] {
            let (x, y, w, h) =
                letterboxed_viewport(800.0 * factor, 600.0 * factor, 800.0, 600.0);
            assert_eq!((x, y), (0.0, 0.0), "facteur {factor}");
            assert_eq!((w, h), (800.0 * factor, 600.0 * factor), "facteur {factor}");
        }
    }

    /// Fenêtre plus large que 4:3 (ex. écran 16:9 en plein écran) : bandes
    /// verticales (pillarboxing), image collée en haut, centrée en largeur.
    #[test]
    fn wider_than_4_3_pillarboxes() {
        let (x, y, w, h) = letterboxed_viewport(1920.0, 1080.0, 800.0, 600.0);
        // Échelle limitée par la hauteur : 1080 / 600 = 1.8
        assert_eq!((w, h), (1440.0, 1080.0));
        assert_eq!(y, 0.0);
        assert!((x - 240.0).abs() < 0.01); // (1920 - 1440) / 2
    }

    /// Fenêtre plus étroite que 4:3 (portrait) : bandes horizontales
    /// (letterboxing), l'inverse du cas précédent.
    #[test]
    fn narrower_than_4_3_letterboxes() {
        let (x, y, w, h) = letterboxed_viewport(600.0, 1200.0, 800.0, 600.0);
        // Échelle limitée par la largeur : 600 / 800 = 0.75
        assert_eq!((w, h), (600.0, 450.0));
        assert_eq!(x, 0.0);
        assert!((y - 375.0).abs() < 0.01); // (1200 - 450) / 2
    }

    /// Fenêtre réduite à rien (minimisée, ou avant la première mesure) : pas
    /// de division par zéro ni de viewport négatif qui ferait paniquer wgpu.
    #[test]
    fn zero_size_surface_does_not_panic() {
        let (_, _, w, h) = letterboxed_viewport(0.0, 0.0, 800.0, 600.0);
        assert_eq!((w, h), (0.0, 0.0));
    }
}
