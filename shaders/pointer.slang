// The layer texture's alpha at the pointer's place, and the share of a half's width at which the
// head camera's ray through the pointer meets the scene in the left eye's picture.
// The target is one texel. The layer spans an entire frame, and each half of the distance texture
// spans one eye's entire picture, so a share of the frame is the same share of each half.

Texture2D<float4> layer : register(t0);
Texture2D<float> distances : register(t1);

// The layout of `Constants` in crates/gfx/src/raw/pointer.rs.
cbuffer Pointer : register(b0)
{
    // The pointer's place as shares of the frame's width and height.
    float2 place;
    // The ray's distance in the distance texture's unit at the share t is ray / t.
    float ray;
    // The largest share the march visits.
    float reach;
};

// One triangle that covers the viewport, built from the vertex index, so the draw binds no vertex buffer.
[shader("vertex")]
float4 vertex_main(uint index : SV_VertexID) : SV_Position
{
    float2 uv = float2((index << 1) & 2, index & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

// The texel of a place `share` of the way across `size` texels.
uint texel_at(float share, uint size)
{
    return min(uint(share * float(size)), size - 1);
}

// The red channel is the hit share and the green channel the layer's alpha, the layout `pointed` in
// crates/gfx/src/pointer.rs reads. The march visits the left half's places `place.x + share` from the
// largest share up to `reach` down to one texel, skips a place past the half, and stops at the first
// place whose distance is at most the ray's. A march with no hit gives the share 0.
[shader("pixel")]
float2 pixel_main(float4 position : SV_Position) : SV_Target
{
    uint layer_width;
    uint layer_height;
    layer.GetDimensions(layer_width, layer_height);
    uint distance_width;
    uint distance_height;
    distances.GetDimensions(distance_width, distance_height);
    uint half_width = distance_width / 2;
    float alpha = layer.Load(int3(texel_at(place.x, layer_width), texel_at(place.y, layer_height), 0)).a;
    uint row = texel_at(place.y, distance_height);
    uint steps = uint(reach * float(half_width));
    float hit = 0.0;
    for (uint n = steps; n > 0; n--)
    {
        float share = float(n) / float(half_width);
        float across = place.x + share;
        if (across >= 1.0)
        {
            continue;
        }
        float distance = distances.Load(int3(texel_at(across, half_width), row, 0));
        if (distance <= ray / share)
        {
            hit = share;
            break;
        }
    }
    return float2(hit, alpha);
}
