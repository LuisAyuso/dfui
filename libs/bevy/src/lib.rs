//! A dfui app as an overlay over a Bevy game: drawn with Bevy's wgpu device
//! into an `Image` shown as a full-window UI node, fed the window's raw winit
//! events.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{Extent3d, Texture, TextureFormat};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::window::PrimaryWindow;
use bevy::winit::RawWinitWindowEvent;
use dfui::{App as DfuiApp, Embedded};

/// Runs the dfui app that `make` builds over the primary window.
pub struct DfuiPlugin<A> {
    make: Mutex<Option<Box<dyn FnOnce() -> A + Send>>>,
}

impl<A: DfuiApp> DfuiPlugin<A> {
    pub fn new(make: impl FnOnce() -> A + Send + 'static) -> Self {
        DfuiPlugin {
            make: Mutex::new(Some(Box::new(make))),
        }
    }
}

/// The running app, a non-send resource: read it with `NonSendMut<Dfui<A>>`
/// and change it with [`Embedded::app`].
pub struct Dfui<A: DfuiApp> {
    ui: Embedded<A>,
    image: Handle<Image>,
    size: (u32, u32),
    scale: f32,
}

impl<A: DfuiApp> std::ops::Deref for Dfui<A> {
    type Target = Embedded<A>;
    fn deref(&self) -> &Embedded<A> {
        &self.ui
    }
}

impl<A: DfuiApp> std::ops::DerefMut for Dfui<A> {
    fn deref_mut(&mut self) -> &mut Embedded<A> {
        &mut self.ui
    }
}

/// Whether the UI took input this frame, so game systems should ignore it.
#[derive(Resource, Default)]
pub struct UiTookInput(pub bool);

#[derive(Resource)]
struct Make<A>(Mutex<Option<Box<dyn FnOnce() -> A + Send>>>);

/// The overlay image and, once the render world made it, its texture.
#[derive(Resource, Clone, Default)]
struct Shared(Arc<Mutex<(AssetId<Image>, Option<Texture>)>>);

const FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

impl<A: DfuiApp> Plugin for DfuiPlugin<A> {
    fn build(&self, app: &mut App) {
        let make = self.make.lock().unwrap().take();
        let shared = Shared::default();
        app.insert_resource(Make(Mutex::new(make)))
            .insert_resource(shared.clone())
            .init_resource::<UiTookInput>()
            .add_systems(Startup, setup::<A>)
            .add_systems(PreUpdate, input::<A>)
            .add_systems(Last, render::<A>);
        app.sub_app_mut(RenderApp)
            .insert_resource(shared)
            .add_systems(Render, share.in_set(RenderSystems::PrepareResources));
    }
}

fn setup<A: DfuiApp>(world: &mut World) {
    let make = world.resource::<Make<A>>().0.lock().unwrap().take();
    let Some(make) = make else { return };
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .expect("dfui-bevy draws over the primary window");
    let size = (
        window.physical_width().max(1),
        window.physical_height().max(1),
    );
    let scale = window.scale_factor();
    let image = world
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(size.0, size.1, FORMAT, None));
    world.resource::<Shared>().0.lock().unwrap().0 = image.id();
    world.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ImageNode::new(image.clone()),
        GlobalZIndex(i32::MAX),
    ));
    let device = world.resource::<RenderDevice>().wgpu_device().clone();
    let queue = (**world.resource::<RenderQueue>()).clone();
    let ui = Embedded::new(make(), device, queue, FORMAT, size, scale);
    world.insert_non_send(Dfui {
        ui,
        image,
        size,
        scale,
    });
}

// ponytail: every window's events go to the one overlay; match `window_id` for multiple windows.
fn input<A: DfuiApp>(
    mut events: MessageReader<RawWinitWindowEvent>,
    mut dfui: NonSendMut<Dfui<A>>,
    mut took: ResMut<UiTookInput>,
) {
    took.0 = false;
    for e in events.read() {
        took.0 |= dfui.ui.window_event(e.event.clone());
    }
}

fn render<A: DfuiApp>(
    mut dfui: NonSendMut<Dfui<A>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    shared: Res<Shared>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let size = (
        window.physical_width().max(1),
        window.physical_height().max(1),
    );
    let scale = window.scale_factor();
    if (size, scale) != (dfui.size, dfui.scale) {
        (dfui.size, dfui.scale) = (size, scale);
        dfui.ui.resize(size, scale);
        if let Some(mut image) = images.get_mut(&dfui.image) {
            image.resize(Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            });
        }
    }
    let Some(texture) = shared.0.lock().unwrap().1.clone() else {
        return;
    };
    // The render world makes the resized texture a frame or so later.
    if (texture.width(), texture.height()) != size {
        return;
    }
    let view = texture.create_view(&Default::default());
    let mut enc = device
        .wgpu_device()
        .create_command_encoder(&Default::default());
    dfui.ui.render(&mut enc, &view);
    queue.submit([enc.finish()]);
}

fn share(images: Res<RenderAssets<GpuImage>>, shared: Res<Shared>) {
    let mut s = shared.0.lock().unwrap();
    s.1 = images.get(s.0).map(|g| g.texture.clone());
}
