//! Renders a generated map to a PPM image (tile map colours, resources highlighted):
//!
//!     cargo run --release -p factorio-data --example mapimage -- out.ppm [radius_chunks] [seed]

use std::io::Write;
use std::time::Instant;

use factorio_sim::map::{CHUNK_SIZE, ChunkPosition, TilePosition};
use factorio_sim::surface::Surface;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or("map.ppm".into());
    let r: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0x5EED);
    let config = factorio_data::Config::load()?;
    let data = factorio_data::load_game_data(&config)?;
    let db = factorio_data::typed::build_prototype_db(&data)?;
    let start = Instant::now();
    let mut surface = Surface::new(factorio_data::mapgen::planet_mapgen(&data, &db, seed));
    if let Some(e) = &surface.noise_error {
        eprintln!("noise error: {e}");
    }
    eprintln!("compiled in {:.2?}", start.elapsed());
    let start = Instant::now();
    for cy in -r..r {
        for cx in -r..r {
            surface.ensure_chunk(ChunkPosition { x: cx, y: cy });
        }
    }
    let n = (2 * r * r * 2) as f64;
    eprintln!(
        "{} chunks in {:.2?} ({:.2} ms/chunk)",
        4 * r * r,
        start.elapsed(),
        start.elapsed().as_secs_f64() * 1000.0 / n * 2.0
    );
    let size = 2 * r * CHUNK_SIZE;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out)?);
    writeln!(f, "P6 {size} {size} 255")?;
    let mut counts = std::collections::BTreeMap::<String, u32>::new();
    for y in 0..size {
        for x in 0..size {
            let t = TilePosition::new(x - size / 2, y - size / 2);
            let tile = surface.tile(t).unwrap();
            let mut c = db.tile(tile).map_color;
            *counts.entry(db.tile(tile).name.clone()).or_default() += 1;
            if let Some(res) = surface.resource(t) {
                let name = &db.entity(res.proto).name;
                *counts.entry(name.clone()).or_default() += 1;
                c = match name.as_str() {
                    "iron-ore" => [104, 132, 146],
                    "copper-ore" => [203, 97, 53],
                    "coal" => [10, 10, 10],
                    "stone" => [176, 154, 108],
                    "uranium-ore" => [0, 230, 0],
                    _ => [255, 0, 255],
                };
            }
            if x == size / 2 || y == size / 2 {
                c = [255, 255, 255];
            }
            f.write_all(&c)?;
        }
    }
    for (k, v) in counts {
        eprintln!("{k}: {v}");
    }
    Ok(())
}
