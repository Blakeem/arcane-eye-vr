//! The GLB fixtures of the tests: JSON and binary data put together into the GLB container.

use png::{BitDepth, ColorType, Encoder};
use serde_json::{Value, json};

pub(crate) const UNSIGNED_BYTE: u32 = 5121;
pub(crate) const SHORT: u32 = 5122;
pub(crate) const UNSIGNED_SHORT: u32 = 5123;
pub(crate) const UNSIGNED_INT: u32 = 5125;
pub(crate) const FLOAT: u32 = 5126;

/// The positions of a counterclockwise triangle in the XY plane.
pub(crate) const TRIANGLE: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// A GLB under construction: its JSON and the data of its binary chunk.
pub(crate) struct Glb {
    pub(crate) json: Value,
    pub(crate) binary: Vec<u8>,
}

impl Glb {
    pub(crate) fn new() -> Self {
        Self { json: json!({ "asset": { "version": "2.0" } }), binary: Vec::new() }
    }

    /// The fixture of one node whose mesh has the one primitive `primitive`.
    pub(crate) fn with_primitive(build: impl FnOnce(&mut Self) -> Value) -> Self {
        let mut glb = Self::new();
        let primitive = build(&mut glb);
        glb.set("meshes", json!([{ "primitives": [primitive] }]));
        glb.set("nodes", json!([{ "mesh": 0 }]));
        glb.set("scenes", json!([{ "nodes": [0] }]));
        glb.set("scene", json!(0));
        glb
    }

    /// A buffer view of `bytes`, which start 4-byte aligned in the binary chunk.
    pub(crate) fn view(&mut self, bytes: &[u8], stride: Option<usize>) -> usize {
        self.binary.resize(self.binary.len().next_multiple_of(4), 0);
        let mut view =
            json!({ "buffer": 0, "byteOffset": self.binary.len(), "byteLength": bytes.len() });
        if let Some(stride) = stride {
            view["byteStride"] = json!(stride);
        }
        self.binary.extend_from_slice(bytes);
        self.push("bufferViews", view)
    }

    pub(crate) fn accessor(
        &mut self,
        view: usize,
        component_type: u32,
        count: usize,
        kind: &str,
    ) -> usize {
        let accessor = json!({ "bufferView": view, "componentType": component_type, "count": count, "type": kind });
        self.push("accessors", accessor)
    }

    /// A tightly packed float accessor of `values`, `components` of them in each element.
    pub(crate) fn floats(&mut self, values: &[f32], components: usize, kind: &str) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        let view = self.view(&bytes, None);
        let count = values.len().checked_div(components).expect("components above 0");
        self.accessor(view, FLOAT, count, kind)
    }

    /// An index accessor of `bytes`, `count` indices of `component_type`.
    pub(crate) fn indices(&mut self, bytes: &[u8], component_type: u32, count: usize) -> usize {
        let view = self.view(bytes, None);
        self.accessor(view, component_type, count, "SCALAR")
    }

    /// An image of `bytes` in a buffer view, with the MIME type `mime_type`.
    pub(crate) fn image(&mut self, bytes: &[u8], mime_type: &str) -> usize {
        let view = self.view(bytes, None);
        self.push("images", json!({ "bufferView": view, "mimeType": mime_type }))
    }

    /// Appends `item` to the top-level array `key` and gives its index.
    pub(crate) fn push(&mut self, key: &str, item: Value) -> usize {
        let object = self.json.as_object_mut().expect("the JSON is an object");
        let array =
            object.entry(key).or_insert_with(|| json!([])).as_array_mut().expect("an array");
        let index = array.len();
        array.push(item);
        index
    }

    pub(crate) fn set(&mut self, key: &str, value: Value) {
        self.json[key] = value;
    }

    /// The GLB file, with one buffer of the binary data unless the JSON lists its own buffers.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut json = self.json.clone();
        if !self.binary.is_empty() && json.get("buffers").is_none() {
            json["buffers"] = json!([{ "byteLength": self.binary.len() }]);
        }
        let text = serde_json::to_vec(&json).expect("the JSON serializes");
        let mut chunks = vec![(*b"JSON", text)];
        if !self.binary.is_empty() {
            chunks.push((*b"BIN\0", self.binary.clone()));
        }
        container(&chunks)
    }
}

/// A GLB of the chunks of each type and data, the JSON padded with spaces and the rest with zeros.
pub(crate) fn container(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, data) in chunks {
        let padding = if kind == b"JSON" { b' ' } else { 0 };
        let mut data = data.clone();
        data.resize(data.len().next_multiple_of(4), padding);
        body.extend_from_slice(&u32::try_from(data.len()).expect("a small chunk").to_le_bytes());
        body.extend_from_slice(kind);
        body.extend_from_slice(&data);
    }
    let length =
        u32::try_from(body.len().checked_add(12).expect("a small file")).expect("a small file");
    let mut file = b"glTF".to_vec();
    file.extend_from_slice(&2_u32.to_le_bytes());
    file.extend_from_slice(&length.to_le_bytes());
    file.extend_from_slice(&body);
    file
}

/// The PNG of `width` by `height` pixels of 8-bit `color` samples.
pub(crate) fn png(width: u32, height: u32, color: ColorType, samples: &[u8]) -> Vec<u8> {
    png_of_depth(width, height, color, BitDepth::Eight, samples)
}

pub(crate) fn png_of_depth(
    width: u32,
    height: u32,
    color: ColorType,
    depth: BitDepth,
    samples: &[u8],
) -> Vec<u8> {
    let mut file = Vec::new();
    let mut encoder = Encoder::new(&mut file, width, height);
    encoder.set_color(color);
    encoder.set_depth(depth);
    let mut writer = encoder.write_header().expect("the PNG header");
    writer.write_image_data(samples).expect("the PNG data");
    writer.finish().expect("the PNG end");
    file
}

/// The PNG of one row of 8-bit palette `indices`, with the transparency `alphas` of the first entries.
pub(crate) fn palette_png(palette: &[u8], alphas: &[u8], indices: &[u8]) -> Vec<u8> {
    let mut file = Vec::new();
    let width = u32::try_from(indices.len()).expect("a short row");
    let mut encoder = Encoder::new(&mut file, width, 1);
    encoder.set_color(ColorType::Indexed);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_palette(palette.to_vec());
    encoder.set_trns(alphas.to_vec());
    let mut writer = encoder.write_header().expect("the PNG header");
    writer.write_image_data(indices).expect("the PNG data");
    writer.finish().expect("the PNG end");
    file
}

/// A baseline JPEG of 8 by 8 pixels of the color [`JPEG_COLOR`], three components without subsampling.
pub(crate) const COLOR_JPEG: &[u8] = include_bytes!("jpeg/color-8x8.jpg");
pub(crate) const JPEG_COLOR: [u8; 3] = [200, 60, 30];
/// A baseline JPEG of 8 by 8 pixels with an Adobe marker of four CMYK components without subsampling,
/// cyan 0, magenta 179, yellow 217 and black 55, which by the naive product of the inverted inks is
/// [`JPEG_COLOR`].
pub(crate) const CMYK_JPEG: &[u8] = include_bytes!("jpeg/cmyk-8x8.jpg");
/// A baseline JPEG of 8 by 8 pixels of the gray [`JPEG_GRAY`], one component.
pub(crate) const GRAY_JPEG: &[u8] = include_bytes!("jpeg/gray-8x8.jpg");
pub(crate) const JPEG_GRAY: u8 = 90;

/// The bytes of `jpeg` before its baseline start of frame marker, and the bytes from the marker on.
fn split_at_frame(jpeg: &[u8]) -> (&[u8], &[u8]) {
    let marker = jpeg.windows(2).position(|pair| pair == [0xFF, 0xC0]).expect("a baseline JPEG");
    jpeg.split_at(marker)
}

/// `jpeg` with the start of frame naming `width` by `height` pixels.
pub(crate) fn jpeg_of_size(jpeg: &[u8], width: u16, height: u16) -> Vec<u8> {
    let (before, frame) = split_at_frame(jpeg);
    let [0xFF, 0xC0, length_high, length_low, precision, _, _, _, _, after_size @ ..] = frame
    else {
        panic!("a start of frame of a height and a width")
    };
    let head = [0xFF, 0xC0, *length_high, *length_low, *precision];
    [before, &head, &height.to_be_bytes(), &width.to_be_bytes(), after_size].concat()
}

/// The one-component `jpeg` with a second component of the first's sampling and table in its start of
/// frame.
pub(crate) fn two_component_jpeg(jpeg: &[u8]) -> Vec<u8> {
    let (before, frame) = split_at_frame(jpeg);
    let (start_of_frame, after) = frame.split_at(13);
    let [0xFF, 0xC0, 0, 11, precision, size @ .., 1, id, sampling, table] = start_of_frame else {
        panic!("a start of frame of one component")
    };
    // The segment length grows from 11 to 14 bytes, and the component count from 1 to 2.
    let head = [0xFF, 0xC0, 0, 14, *precision];
    let components = [2, *id, *sampling, *table, id.wrapping_add(1), *sampling, *table];
    [before, &head, size, &components, after].concat()
}

/// The fixture of one node with a textured triangle whose material names the image `image`.
pub(crate) fn textured(image: impl FnOnce(&mut Glb) -> usize) -> Glb {
    let mut glb = Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        json!({ "attributes": { "POSITION": position }, "material": 0 })
    });
    let source = image(&mut glb);
    glb.set("textures", json!([{ "source": source }]));
    let pbr =
        json!({ "baseColorFactor": [0.5, 0.25, 1.0, 1.0], "baseColorTexture": { "index": 0 } });
    glb.set("materials", json!([{ "pbrMetallicRoughness": pbr }]));
    glb
}
