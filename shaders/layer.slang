// Samples one texture over the entire viewport. The blend state the draw binds combines it with the target.

Texture2D<float4> layer : register(t0);
SamplerState layer_sampler : register(s0);

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
    return layer.Sample(layer_sampler, corner.uv);
}
