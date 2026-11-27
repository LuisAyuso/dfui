//! Planets orbiting a sun in Bevy, with a dfui panel over them. Clicks that
//! miss the panel drop a planet into a circular orbit.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, button, column, container, row, slider, spacer, text, toggle};
use dfui::{Color as UiColor, Element, Role};
use dfui_bevy::{Dfui, DfuiPlugin, UiTookInput};

const GM: f32 = 4.0e6;

enum Cmd {
    Add,
    Clear,
    Speed(f32),
    Pause(bool),
}

struct Hud {
    tx: Sender<Cmd>,
    bodies: usize,
    time: f32,
    fps: f64,
    speed: f32,
    paused: bool,
}

impl dfui::App for Hud {
    fn view(&self) -> Element {
        let send = |c: fn() -> Cmd| {
            let tx = self.tx.clone();
            move |_: &mut dfui::EventCx| {
                let _ = tx.send(c());
            }
        };
        let (speed, pause) = (self.tx.clone(), self.tx.clone());
        let panel = column([
            row([
                text("Orbits").size(20.0).into(),
                spacer().grow(1.0).into(),
                text(format!("{:.0} fps", self.fps))
                    .color(Role::Muted)
                    .into(),
            ])
            .align(AlignItems::CENTER)
            .into(),
            text(format!("{} planets · t = {:.0}", self.bodies, self.time))
                .color(Role::Muted)
                .into(),
            row([
                text("Speed").into(),
                slider(self.speed, 0.0..=4.0)
                    .on_change(move |_, v| {
                        let _ = speed.send(Cmd::Speed(v));
                    })
                    .grow(1.0)
                    .into(),
            ])
            .gap(12.0)
            .align(AlignItems::CENTER)
            .into(),
            row([
                text("Paused").into(),
                spacer().grow(1.0).into(),
                toggle(self.paused)
                    .on_change(move |_, on| {
                        let _ = pause.send(Cmd::Pause(on));
                    })
                    .into(),
            ])
            .align(AlignItems::CENTER)
            .into(),
            row([
                button(text("Add planet"))
                    .on_click(send(|| Cmd::Add))
                    .into(),
                button(text("Clear")).on_click(send(|| Cmd::Clear)).into(),
            ])
            .gap(8.0)
            .into(),
            text("Click empty space to drop a planet")
                .size(12.0)
                .color(Role::Muted)
                .into(),
        ])
        .gap(12.0);
        column([container(panel)
            .bg(Role::Surface)
            .radius(12.0)
            .padding(16.0)
            .width(280.0)
            .into()])
        .padding(16.0)
        .into()
    }

    fn background(&self) -> UiColor {
        UiColor::TRANSPARENT
    }
}

#[derive(Resource)]
struct Inbox(Mutex<Receiver<Cmd>>);

#[derive(Resource)]
struct Sim {
    speed: f32,
    paused: bool,
    time: f32,
    added: u32,
    mesh: Handle<Mesh>,
    colors: Vec<Handle<ColorMaterial>>,
}

#[derive(Component)]
struct Planet {
    vel: Vec2,
}

fn main() {
    let (tx, rx) = channel();
    App::new()
        .add_plugins((DefaultPlugins, FrameTimeDiagnosticsPlugin::default()))
        .add_plugins(DfuiPlugin::new(move || Hud {
            tx,
            bodies: 0,
            time: 0.0,
            fps: 0.0,
            speed: 1.0,
            paused: false,
        }))
        .insert_resource(Inbox(Mutex::new(rx)))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.06)))
        .add_systems(Startup, setup)
        .add_systems(Update, (commands, click, step, hud).chain())
        .run();
}

fn setup(
    mut cmds: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    cmds.spawn(Camera2d);
    cmds.spawn((
        Mesh2d(meshes.add(Circle::new(28.0))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 0.8, 0.3))),
    ));
    let colors = [
        (0.4, 0.7, 1.0),
        (0.9, 0.4, 0.3),
        (0.5, 0.9, 0.5),
        (0.8, 0.6, 1.0),
    ]
    .map(|(r, g, b)| materials.add(Color::srgb(r, g, b)))
    .to_vec();
    let mut sim = Sim {
        speed: 1.0,
        paused: false,
        time: 0.0,
        added: 0,
        mesh: meshes.add(Circle::new(7.0)),
        colors,
    };
    for _ in 0..5 {
        add_planet(&mut cmds, &mut sim, None);
    }
    cmds.insert_resource(sim);
}

/// Spawns a planet at `at`, or at the next spot on a spiral, on a circular orbit.
fn add_planet(cmds: &mut Commands, sim: &mut Sim, at: Option<Vec2>) {
    let n = sim.added;
    sim.added += 1;
    let pos = at.unwrap_or_else(|| {
        let r = 90.0 + (n % 6) as f32 * 45.0;
        Vec2::from_angle(n as f32 * 2.4) * r
    });
    let r = pos.length().max(40.0);
    let vel = pos.perp().normalize_or_zero() * (GM / r).sqrt() / 10.0;
    cmds.spawn((
        Planet { vel },
        Mesh2d(sim.mesh.clone()),
        MeshMaterial2d(sim.colors[n as usize % sim.colors.len()].clone()),
        Transform::from_translation(pos.extend(1.0)),
    ));
}

fn commands(
    inbox: Res<Inbox>,
    mut sim: ResMut<Sim>,
    mut cmds: Commands,
    planets: Query<Entity, With<Planet>>,
) {
    for c in inbox.0.lock().unwrap().try_iter() {
        match c {
            Cmd::Add => add_planet(&mut cmds, &mut sim, None),
            Cmd::Clear => planets.iter().for_each(|e| cmds.entity(e).despawn()),
            Cmd::Speed(s) => sim.speed = s,
            Cmd::Pause(p) => sim.paused = p,
        }
    }
}

fn click(
    mouse: Res<ButtonInput<MouseButton>>,
    took: Res<UiTookInput>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut sim: ResMut<Sim>,
    mut cmds: Commands,
) {
    if !mouse.just_pressed(MouseButton::Left) || took.0 {
        return;
    }
    let (camera, at) = *camera;
    if let Some(p) = window.cursor_position()
        && let Ok(p) = camera.viewport_to_world_2d(at, p)
    {
        add_planet(&mut cmds, &mut sim, Some(p));
    }
}

fn step(time: Res<Time>, mut sim: ResMut<Sim>, mut planets: Query<(&mut Planet, &mut Transform)>) {
    if sim.paused {
        return;
    }
    let dt = time.delta_secs() * sim.speed * 10.0;
    sim.time += dt;
    let steps = 8;
    let h = dt / steps as f32;
    for (mut p, mut t) in &mut planets {
        let mut pos = t.translation.truncate();
        for _ in 0..steps {
            let r = pos.length().max(20.0);
            p.vel += -pos / (r * r * r) * GM / 100.0 * h;
            pos += p.vel * h;
        }
        t.translation = pos.extend(1.0);
    }
}

fn hud(
    sim: Res<Sim>,
    diagnostics: Res<DiagnosticsStore>,
    planets: Query<(), With<Planet>>,
    mut ui: NonSendMut<Dfui<Hud>>,
) {
    let bodies = planets.iter().count();
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    ui.app(|h| {
        (h.bodies, h.time, h.speed, h.paused, h.fps) =
            (bodies, sim.time, sim.speed, sim.paused, fps);
    });
}
