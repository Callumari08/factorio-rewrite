//! Power graph for the electric network window, drawn into a texture on the CPU.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use factorio_sim::power::NetworkStats;
use factorio_sim::proto::EntityProtoId;

pub const WIDTH: u32 = 300;
pub const HEIGHT: u32 = 140;

pub const PALETTE: [[u8; 3]; 8] = [
    [235, 190, 60],
    [90, 170, 255],
    [120, 220, 120],
    [240, 110, 90],
    [200, 130, 240],
    [90, 220, 210],
    [240, 160, 200],
    [170, 170, 170],
];
pub const PRODUCTION: [u8; 3] = [255, 255, 255];

#[derive(Resource)]
pub struct Chart {
    pub image: Handle<Image>,
    /// 0 = 5 s, 1 = 1 min, 2 = 10 min.
    pub range: usize,
}

pub struct ChartPlugin;

impl Plugin for ChartPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, setup);
    }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(blank());
    commands.insert_resource(Chart { image, range: 1 });
}

fn blank() -> Image {
    Image::new(
        Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (WIDTH * HEIGHT * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

/// The entity types shown, largest consumers first.
pub fn series_order(stats: &NetworkStats, range: usize) -> Vec<EntityProtoId> {
    let mut totals: std::collections::BTreeMap<EntityProtoId, i64> = Default::default();
    for s in &stats.series[range].samples {
        for (k, v) in &s.consumption {
            *totals.entry(*k).or_insert(0) += v.raw();
        }
    }
    let mut v: Vec<(EntityProtoId, i64)> = totals.into_iter().collect();
    v.sort_by_key(|(k, t)| (-t, *k));
    v.into_iter().map(|(k, _)| k).take(PALETTE.len()).collect()
}

pub fn draw(images: &mut Assets<Image>, chart: &Chart, stats: &NetworkStats) {
    let Some(image) = images.get_mut(&chart.image) else { return };
    let mut px = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    // Background and grid.
    for i in 0..(WIDTH * HEIGHT) as usize {
        px[i * 4..i * 4 + 4].copy_from_slice(&[24, 24, 24, 255]);
    }
    let samples = &stats.series[chart.range].samples;
    let max = samples
        .iter()
        .map(|s| s.total_production().max(s.total_consumption()).to_f64_lossy())
        .fold(0.0f64, f64::max)
        .max(1.0)
        * 1.1;
    let mut set = |x: i32, y: i32, c: [u8; 3]| {
        if x >= 0 && y >= 0 && (x as u32) < WIDTH && (y as u32) < HEIGHT {
            let i = ((y as u32 * WIDTH + x as u32) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    };
    for k in 1..4 {
        let y = (HEIGHT * k / 4) as i32;
        for x in 0..WIDTH as i32 {
            if x % 4 == 0 {
                set(x, y, [60, 60, 60]);
            }
        }
    }
    let n = factorio_sim::power::STAT_SAMPLES;
    let to_xy = |i: usize, v: f64| {
        let offset = n - samples.len();
        let x = ((offset + i) as f64 / (n - 1) as f64 * (WIDTH - 1) as f64) as i32;
        let y = (HEIGHT as f64 - 1.0 - v / max * (HEIGHT - 2) as f64) as i32;
        (x, y)
    };
    let line = |values: Vec<f64>, c: [u8; 3], set: &mut dyn FnMut(i32, i32, [u8; 3])| {
        for i in 1..values.len() {
            let (x0, y0) = to_xy(i - 1, values[i - 1]);
            let (x1, y1) = to_xy(i, values[i]);
            let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
            for s in 0..=steps {
                set(x0 + (x1 - x0) * s / steps, y0 + (y1 - y0) * s / steps, c);
            }
        }
    };
    for (k, proto) in series_order(stats, chart.range).iter().enumerate() {
        let values = samples.iter().map(|s| s.consumption.get(proto).map_or(0.0, |v| v.to_f64_lossy())).collect();
        line(values, PALETTE[k], &mut set);
    }
    let production = samples.iter().map(|s| s.total_production().to_f64_lossy()).collect();
    line(production, PRODUCTION, &mut set);
    image.data = Some(px);
}

/// Formats joules per tick as a power figure.
pub fn power_text(j_per_tick: f64) -> String {
    let w = j_per_tick * 60.0;
    if w >= 1e6 { format!("{:.2} MW", w / 1e6) } else { format!("{:.1} kW", w / 1e3) }
}
