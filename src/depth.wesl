import bevy_sprite_render::{
    mesh2d::vertex_output::VertexOutput,
    mesh2d::view_bindings::view,
    mesh2d::functions as mesh_functions,
    sprite_mesh::bindings::{material, texture},
    sprite_mesh::functions as sprite_functions,
};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
import bevy_render::color_operations::{linear_to_srgb, srgb_to_linear};
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

// Mode, depth shape offset x and y, unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(20) var<uniform> params: vec4<i32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(21) var shape: texture_2d<i32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(22) var<uniform> palette_light: vec4<u32>;

struct Output {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

fn floor_div(a: i32, b: i32) -> i32 {
    var q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q = q - 1;
    }
    return q;
}

fn texel(uv: vec2<f32>, size: vec2<u32>) -> vec2<i32> {
    return vec2<i32>(floor(uv * vec2<f32>(size)));
}

fn palette_rgb(linear: vec3<f32>, light: vec3<u32>) -> vec3<f32> {
    let bytes = vec3<u32>(round(clamp(linear_to_srgb(linear), vec3(0.0), vec3(1.0)) * 255.0));
    // Object LightConvert uses the MMX Q12 path; ordinary ANIM.PAL Convert
    // builds each palette row with the full Q16 factor.
    var shaded = (bytes * (light >> vec3(4u))) >> vec3(12u);
    if params.w == 2 {
        shaded = (bytes * light) >> vec3(16u);
    }
    shaded = min(vec3(255u), shaded);
    let packed = shaded >> vec3(3u, 2u, 3u);
    let expanded = (packed << vec3(3u, 2u, 3u)) | (packed >> vec3(2u, 4u, 2u));
    return srgb_to_linear(vec3<f32>(expanded) / 255.0);
}

@fragment
fn fragment(mesh: VertexOutput) -> Output {
    var base = sprite_functions::sample_sprite_texture(mesh.uv, mesh.instance_index);
    if params.w != 0 {
        base = vec4(palette_rgb(base.rgb, palette_light.rgb), base.a);
    }
    var color = sprite_functions::get_final_color(base, mesh.instance_index);
    if color.a <= 0.0 {
        discard;
    }
    let row = i32(floor(-mesh.world_position.y));
    let adjust = bitcast<i32>(mesh_functions::get_tag(mesh.instance_index));
    let mode = params.x;
    // The bottom row of the picture's own rectangle.
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let bottom = world_from_local * vec4<f32>(0.0, -0.5 * material.vertex_scale.y + material.vertex_offset.y, 0.0, 1.0);
    let yb = i32(round(-bottom.y)) - 1;
    var d: i32;
    if mode == 1 {
        d = 3 * floor_div(-yb + adjust, 3) - floor_div(yb - row, 3);
    } else if mode == 2 {
        d = -row + adjust;
    } else {
        let uv = sprite_functions::get_final_uv(mesh.uv, mesh.instance_index);
        let at = texel(uv, textureDimensions(texture)) + params.yz;
        let size = vec2<i32>(textureDimensions(shape));
        if at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y {
            discard;
        }
        d = -yb + adjust - textureLoad(shape, at, 0).r;
    }

@if(TONEMAP_IN_SHADER)
    color = tonemapping::tone_mapping(color, view.color_grading);
@if(SRGB_OUTPUT)
    color = vec4(linear_to_srgb(color.rgb), color.a);
@if(OKLAB_OUTPUT)
    color = vec4(linear_rgb_to_oklab(color.rgb), color.a);

    // Every depth lies behind world z = -1, so plain sprites draw over all of
    // them as before; nearer is greater.
    let limit = view.clip_from_world * vec4<f32>(0.0, 0.0, -1.0, 1.0);
    var out: Output;
    out.color = color;
    out.depth = limit.z / limit.w * clamp(f32(8192 - d) / 262144.0, 0.0, 1.0);
    return out;
}
