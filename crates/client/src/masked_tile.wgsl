// Ground quads: a picture from a cell of a sheet, optionally multiplied by a mask piece
// from another cell. Each quad carries its cells' UV rects (min.xy, max.xy): the picture's
// in the tangent attribute and the mask's in the colour attribute; its UV is the position
// within the quad, 0..1. Samples are kept inside their cell at the mip level in use, so
// neighbouring cells of the sheet never bleed in when zoomed out.
#import bevy_sprite::{mesh2d_functions as mesh_functions, mesh2d_vertex_output::VertexOutput}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var color_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var color_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var mask_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color_rect: vec4<f32>,
    @location(4) mask_rect: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh2d_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.position = mesh_functions::mesh2d_position_world_to_clip(out.world_position);
    out.uv = vertex.uv;
    out.world_tangent = vertex.color_rect;
    out.color = vertex.mask_rect;
    return out;
}

fn sample_cell(t: texture_2d<f32>, s: sampler, rect: vec4<f32>, local: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let p = mix(rect.xy, rect.zw, local);
    let dx = dpdx(p);
    let dy = dpdy(p);
    let texels = max(length(dx * size), length(dy * size));
    let lod = clamp(log2(max(texels, 1e-6)), 0.0, f32(textureNumLevels(t) - 1u));
    // Half a texel of the (coarser) level in use, in UV.
    let half_texel = 0.5 * exp2(ceil(lod)) / size;
    let lo = min(rect.xy + half_texel, (rect.xy + rect.zw) * 0.5);
    let hi = max(rect.zw - half_texel, (rect.xy + rect.zw) * 0.5);
    return textureSampleGrad(t, s, clamp(p, lo, hi), dx, dy);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let local = clamp(in.uv, vec2(0.0), vec2(1.0));
    var color = sample_cell(color_texture, color_sampler, in.world_tangent, local);
    let m = sample_cell(mask_texture, mask_sampler, in.color, local);
    color.a = color.a * m.r * m.a;
    return color;
}
