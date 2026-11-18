// Copies one rectangle of a texture into the target, one source texel to one target pixel.
// The viewport spans the entire source with the rectangle's top left corner at the target's top left
// corner, so the viewport position of each target pixel names the source texel it shows.

Texture2D<float4> source : register(t0);

struct Corner
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

// One triangle that covers the viewport, built from the vertex index, so the draw binds no vertex buffer.
[shader("vertex")]
Corner vertex_main(uint index : SV_VertexID)
{
    Corner corner;
    corner.uv = float2((index << 1) & 2, index & 2);
    corner.position = float4(corner.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return corner;
}

[shader("pixel")]
float4 pixel_main(Corner corner) : SV_Target
{
    uint width;
    uint height;
    source.GetDimensions(width, height);
    int2 texel = int2(corner.uv * float2(width, height));
    return source.Load(int3(texel, 0));
}
