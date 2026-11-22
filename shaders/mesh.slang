// Lit, textured triangle meshes for the mesh draw of crates/gfx/src/mesh.rs.
// A matrix is stored column-major and transforms a column vector, as mul(matrix, vector), the
// layout that crates/gfx/build.rs compiles with.
// `slangc mesh.slang -target dxbc -profile vs_5_0 -entry vertex_main -matrix-layout-column-major
// -reflection-json <file>`, and the same with ps_5_0 and pixel_main, show the registers that
// crates/gfx/src/raw/mesh.rs binds: View at b0, Item at b1, color at t0 and color_sampler at s0.

// The layout of `ViewConstants` in crates/gfx/src/raw/mesh.rs.
cbuffer View : register(b0)
{
    float4x4 view_projection;
    // The viewer as a homogeneous point of world space: a place for a perspective projection, or the
    // direction toward the viewer with w 0 for a parallel projection.
    float4 viewer;
};

// The layout of `ItemConstants` in crates/gfx/src/raw/mesh.rs.
cbuffer Item : register(b1)
{
    float4x4 model;
    // The base color factor's linear red, green and blue.
    float3 factor;
};

Texture2D<float4> color : register(t0);
SamplerState color_sampler : register(s0);

// The share of the light that reaches every surface, whatever its direction.
static const float AMBIENT = 0.35;
// The share of the light that falls with the cosine of the angle to the viewer.
static const float FACING = 0.65;

// The layout of `Vertex` in crates/gfx/src/mesh.rs, which the input layout names.
struct Vertex
{
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

struct Surface
{
    float4 position : SV_Position;
    float3 world : TEXCOORD1;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

// The normal through the inverse transpose of the model matrix's linear part, which keeps it at a
// right angle to the surface under a scale that differs per axis. The cofactor matrix is that
// inverse transpose times the determinant, so its sign is put back.
float3 world_normal(float3 normal)
{
    float3x3 columns = transpose((float3x3)model);
    float3 x = columns[0];
    float3 y = columns[1];
    float3 z = columns[2];
    float3 cofactor = normal.x * cross(y, z) + normal.y * cross(z, x) + normal.z * cross(x, y);
    return dot(x, cross(y, z)) < 0.0 ? -cofactor : cofactor;
}

[shader("vertex")]
Surface vertex_main(Vertex vertex)
{
    Surface surface;
    float4 world = mul(model, float4(vertex.position, 1.0));
    surface.position = mul(view_projection, world);
    surface.world = world.xyz;
    surface.normal = world_normal(vertex.normal);
    surface.uv = vertex.uv;
    return surface;
}

// The cosine of the angle between two directions, or 1 when either has no length, such as a mesh
// vertex whose normal is zero.
float cosine(float3 a, float3 b)
{
    float lengths = length(a) * length(b);
    return lengths > 0.0 ? dot(a, b) / lengths : 1.0;
}

// The output is opaque, so its color is premultiplied as it is.
[shader("pixel")]
float4 pixel_main(Surface surface) : SV_Target
{
    float3 toward_viewer = viewer.xyz - surface.world * viewer.w;
    float light = AMBIENT + FACING * max(cosine(surface.normal, toward_viewer), 0.0);
    float3 base = factor * color.Sample(color_sampler, surface.uv).rgb;
    return float4(base * light, 1.0);
}
