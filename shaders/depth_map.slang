// The uncrossed disparity of what each cell of the left half of a frame shows, and whether that is
// the interface, the scene or a covered scene.
// The target holds one pixel per cell, so the pixel names the cell. The layer spans an entire
// frame, and each half of the distance texture spans one eye's entire picture, so a share of the
// left half is the same share of the distance texture's left half.

Texture2D<float4> layer : register(t0);
Texture2D<float> distances : register(t1);

// The layout of `Constants` in crates/gfx/src/raw/depth_map.rs.
cbuffer Map : register(b0)
{
    // The crossed disparity of the interface between the two halves. The UI draw moves the layer
    // half of it to the right in the left half.
    float ui_disparity;
    // The crossed disparity of a scene at the distance texture's value 1, so a value v has the
    // crossed disparity crossed / v.
    float crossed;
    // The depth offset in use, the uncrossed disparity of a scene at an endless distance.
    float offset;
    // 1 when the frame draws the layer, else 0.
    uint ui_drawn;
    // 1 when the distance texture gives the scene's disparity, else 0 for the pair the core makes
    // from a flat picture.
    uint scene_rendered;
    uint columns;
    uint rows;
    // The interface area's width and height as shares of the half, centered in it, which the UI
    // draw put the layer into.
    float area_width;
    float area_height;
};

// A layer texel covers the scene when its alpha is above this value, as `COVERING` in
// shaders/nearest.slang and in crates/arcane-eye-core/src/ui_layer/ui_depth/decide.rs decides.
static const float COVERING = 0.25;

// The kind values `INTERFACE`, `SCENE` and `COVERED_SCENE` of
// crates/ipc-layout/src/viewer_header/depth_map.rs.
static const float INTERFACE = 1.0;
static const float SCENE = 0.0;
static const float COVERED_SCENE = 2.0;

// The exponent bits of an f32, all set for an endless value and a NaN.
static const uint EXPONENT = 0x7F800000;

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

// Whether the frame drew the layer into an area that is not the entire half.
bool area_scaled()
{
    return area_width < 1.0 || area_height < 1.0;
}

// Whether the layer texel at the game place `game` covers the scene. A place off the layer covers
// nothing.
bool covers(float2 game)
{
    if (game.x < 0.0 || game.x >= 1.0 || game.y < 0.0 || game.y >= 1.0)
    {
        return false;
    }
    uint layer_width;
    uint layer_height;
    layer.GetDimensions(layer_width, layer_height);
    int3 texel = int3(texel_at(game.x, layer_width), texel_at(game.y, layer_height), 0);
    return layer.Load(texel).a > COVERING;
}

// The interface's cell at `place` when a covering layer texel shows there, else a cell whose kind
// is not the interface. The UI draw put the layer's game place g at the place 0.5 + (g - 0.5) times
// the area's share, moved right by half the UI disparity. An area of the entire half takes no
// division, so its cells keep the texels of a draw over the entire half.
float2 interface_at(float2 place)
{
    if (ui_drawn == 0)
    {
        return float2(0.0, SCENE);
    }
    float2 drawn = float2(place.x - ui_disparity / 2.0, place.y);
    float2 game = area_scaled() ? 0.5 + (drawn - 0.5) / float2(area_width, area_height) : drawn;
    return covers(game) ? float2(-ui_disparity, INTERFACE) : float2(0.0, SCENE);
}

// The scene's cell at `place`. The exponent test, not a float compare, finds a value that is not
// finite, since the compiler may drop a NaN compare.
float2 scene_at(float2 place)
{
    if (scene_rendered == 0)
    {
        return float2(0.0, SCENE);
    }
    uint distance_width;
    uint distance_height;
    distances.GetDimensions(distance_width, distance_height);
    uint half_width = distance_width / 2;
    float distance = distances.Load(int3(texel_at(place.x, half_width), texel_at(place.y, distance_height), 0));
    if ((asuint(distance) & EXPONENT) == EXPONENT || distance <= 0.0)
    {
        return float2(offset, SCENE);
    }
    return float2(offset - crossed / distance, SCENE);
}

// The kind of the scene at `place` whose uncrossed disparity is `disparity`: a covered scene when
// the frame drew the layer into an area that is not the entire half and the layer texel at the
// scene's game place covers the scene, since the game gives a click there to that texel's interface
// element. The game place is the head camera's place of the scene, midway between its places in the
// two halves, less half the depth offset by which the core moved the right half's scene.
float kind_of_scene(float2 place, float disparity)
{
    float2 game = float2(place.x + (disparity - offset) / 2.0, place.y);
    return (ui_drawn != 0 && area_scaled() && covers(game)) ? COVERED_SCENE : SCENE;
}

// The red channel is the uncrossed disparity and the green channel the kind, the cell layout of
// crates/ipc-layout/src/viewer_header/depth_map.rs. The pixel's middle is the cell's middle.
[shader("pixel")]
float2 pixel_main(float4 position : SV_Position) : SV_Target
{
    float2 place = position.xy / float2(float(columns), float(rows));
    float2 shown = interface_at(place);
    if (shown.y == INTERFACE)
    {
        return shown;
    }
    float disparity = scene_at(place).x;
    return float2(disparity, kind_of_scene(place, disparity));
}
