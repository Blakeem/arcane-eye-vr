// The smallest distance under the covering texels of one column of a grid over a layer texture.
// The target is one row with one pixel per grid column, so the pixel's column names the grid column.
// The layer spans an entire frame, and each half of the distance texture spans one eye's entire
// picture, so a layer texel's place maps to the same place in each half.

Texture2D<float4> layer : register(t0);
Texture2D<float> distances : register(t1);

// The texels between two grid points in each direction. `SPACING` in crates/gfx/src/nearest.rs sizes
// the target with the same value.
static const uint SPACING = 8;

// A layer texel covers the scene when its alpha is above this value.
static const float COVERING = 0.25;

// The column's value when no texel of it covers, below every distance.
static const float NOTHING = -1.0;

// One triangle that covers the viewport, built from the vertex index, so the draw binds no vertex buffer.
[shader("vertex")]
float4 vertex_main(uint index : SV_VertexID) : SV_Position
{
    float2 uv = float2((index << 1) & 2, index & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

// The texel of a place `share` of the way across `size` texels that start at `first`.
uint texel_at(float share, uint first, uint size)
{
    return first + min(uint(share * float(size)), size - 1);
}

[shader("pixel")]
float pixel_main(float4 position : SV_Position) : SV_Target
{
    uint layer_width;
    uint layer_height;
    layer.GetDimensions(layer_width, layer_height);
    uint distance_width;
    uint distance_height;
    distances.GetDimensions(distance_width, distance_height);
    uint left_width = distance_width / 2;
    uint right_width = distance_width - left_width;
    uint x = uint(position.x) * SPACING;
    float across = (float(x) + 0.5) / float(layer_width);
    uint left_x = texel_at(across, 0, left_width);
    uint right_x = texel_at(across, left_width, right_width);
    float nearest = NOTHING;
    for (uint y = 0; y < layer_height; y += SPACING)
    {
        if (layer.Load(int3(x, y, 0)).a <= COVERING)
        {
            continue;
        }
        uint row = texel_at((float(y) + 0.5) / float(layer_height), 0, distance_height);
        float left = distances.Load(int3(left_x, row, 0));
        float right = distances.Load(int3(right_x, row, 0));
        float pair = min(left, right);
        nearest = nearest < 0.0 ? pair : min(nearest, pair);
    }
    return nearest;
}
