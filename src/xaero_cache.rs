// SPDX-License-Identifier: GPL-3.0-only
use crate::runtime::maps::{Layer, Tile};
use std::{io, sync::Arc};

const FORMAT: i32 = (3 << 16) | 25;
pub const MAX_REGION_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_BLOCK_REGION_BYTES: u64 = 64 * 1024 * 1024;

pub struct Region {
    pub layer: Layer,
    pub tiles: Vec<([i32; 2], Tile)>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Bytes<'a>(&'a [u8]);

impl<'a> Bytes<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let (bytes, rest) = self.0.split_at_checked(count).ok_or_else(|| invalid("Truncated Xaero cache"))?;
        self.0 = rest;
        Ok(bytes)
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn int(&mut self) -> io::Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn short(&mut self) -> io::Result<usize> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()).into())
    }

    fn count(&mut self, max: usize) -> io::Result<usize> {
        match self.int()? {
            -1 => Ok(0),
            n if n >= 0 && n as usize <= max => Ok(n as usize),
            _ => Err(invalid("Xaero cache palette is invalid")),
        }
    }
}

fn coordinates(index: u8) -> io::Result<[i32; 2]> {
    let [x, z] = [index >> 4, index & 15];
    if x >= 8 || z >= 8 {
        return Err(invalid("Xaero cache texture coordinates are invalid"));
    }
    Ok([x.into(), z.into()])
}

// The format stores four 13-bit values per big-endian long, from its low bits.
fn packed(values: &[u8], index: usize) -> u16 {
    let start = index / 4 * 8;
    let number = u64::from_be_bytes(values[start..start + 8].try_into().unwrap());
    ((number >> ((index % 4) * 13)) & 8191) as u16
}

pub fn region(bytes: &[u8], coordinate: [i32; 2], cave: Option<i32>) -> io::Result<Region> {
    if coordinate.iter().any(|n| n.unsigned_abs() > 58_594) {
        return Err(invalid("Xaero cache region is outside the world"));
    }
    let mut input = Bytes(bytes);
    let version = input.int()?;
    if !(((3 << 16) | 22)..=FORMAT).contains(&version) {
        return Err(invalid("Unsupported Xaero texture cache version (expected 3.22–3.25)"));
    }
    let minor = version & 65535;
    input.take(12)?; // cache, reload and highlight revisions
    let cave_top = if minor >= 23 { Some(input.int()?) } else { None };
    if minor >= 24 { input.int()?; }
    let layer = match cave {
        None => Layer::Surface,
        Some(hint) => cave_layer(cave_top, hint)?,
    };

    for _ in 0..=64 {
        let marker = input.byte()?;
        if marker == 255 {
            break;
        }
        coordinates(marker)?;
        input.int()?;
    }
    let mut biomes = Vec::new();
    for _ in 0..input.count(16_384)? {
        let biome = match input.byte()? {
            255 => None,
            0 => {
                let size = input.short()?;
                let name = std::str::from_utf8(input.take(size)?).ok();
                name.filter(|s| s.len() <= 128 && s.contains(':')).map(Arc::<str>::from)
            }
            1 => {
                input.int()?; // Legacy numeric biome ID with no namespaced mapping here.
                None
            }
            _ => return Err(invalid("Unsupported Xaero biome palette entry")),
        };
        biomes.push(biome);
    }
    let mut tiles = Vec::new();
    let mut seen = [false; 64];
    loop {
        let marker = input.byte()?;
        if marker == 255 {
            break;
        }
        let [tx, tz] = coordinates(marker)?;
        if std::mem::replace(&mut seen[(tx * 8 + tz) as usize], true) {
            return Err(invalid("Duplicate Xaero cache texture"));
        }
        let colors = input.take(64 * 64 * 4)?;
        let lit = input.byte()?;
        if lit > 1 {
            return Err(invalid("Xaero cache light flag is invalid"));
        }
        let _heights = input.take(1024 * 8)?;
        let top = input.take(1024 * 8)?;
        let mut palette = Vec::new();
        for _ in 0..input.count(4096)? {
            let entry = input.int()?;
            if entry == -1 {
                palette.push(None);
            } else {
                let count = input.short()?;
                if count > 4096 {
                    return Err(invalid("Xaero cache biome count is invalid"));
                }
                palette.push(biomes.get(entry as usize).ok_or_else(|| invalid("Xaero cache biome index is invalid"))?.clone());
            }
        }
        let indices = match palette.len() {
            0 => None,
            _ => match input.byte()? {
                0 if palette.len() == 1 => None,
                1 => Some(input.take(1024 * 8)?),
                _ => return Err(invalid("Xaero cache biome storage is invalid")),
            },
        };

        for dz in 0..4 {
            for dx in 0..4 {
                let mut tile = Tile { pixels: vec![0; 1024], ..Tile::default() };
                tile.biomes.resize(256, None);
                let mut known = false;
                for z in 0..16 {
                    for x in 0..16 {
                        let i = (dz * 16 + z) * 64 + dx * 16 + x;
                        let j = z * 16 + x;
                        let height = packed(top, i);
                        let [red, green, blue, alpha] = colors[i * 4..i * 4 + 4].try_into().unwrap();
                        if alpha == 0 && height & 4096 == 0 {
                            continue;
                        }
                        known = true;
                        tile.pixels[j * 4..j * 4 + 4].copy_from_slice(&[red, green, blue, 255]);
                        tile.heights[j] = if height & 4096 != 0 {
                            (i32::from(height & 4095) << 20) >> 20
                        } else {
                            i32::MIN
                        };
                        let index = indices.map_or(0, |entries| usize::from(packed(entries, i)));
                        tile.biomes[j] = palette.get(index).cloned().flatten();
                    }
                }
                if known {
                    tiles.push(([coordinate[0] * 32 + tx * 4 + dx as i32, coordinate[1] * 32 + tz * 4 + dz as i32], tile));
                }
            }
        }
    }
    if !input.0.is_empty() {
        return Err(invalid("Unexpected data after Xaero cache textures"));
    }
    Ok(Region { layer, tiles })
}

fn cave_layer(height: Option<i32>, hint: i32) -> io::Result<Layer> {
    let height = height.filter(|height| *height != i32::MAX).unwrap_or(hint);
    (-4096..=4096).contains(&height).then_some(Layer::Below(height))
        .ok_or_else(|| invalid("Xaero cave cache height is invalid"))
}

#[derive(Clone)]
pub struct SavedBlock {
    pub name: Arc<str>,
    pub color: u32,
    pub air: bool,
}

fn palette_block(
    input: &mut Bytes<'_>,
    palette: &mut Vec<SavedBlock>,
    inline: bool,
    decode: &impl Fn(&[u8]) -> io::Result<(SavedBlock, usize)>,
) -> io::Result<SavedBlock> {
    if !inline {
        return palette.get(input.int()? as usize).cloned()
            .ok_or_else(|| invalid("Unknown Xaero block palette index"));
    }
    if palette.len() >= 262_144 {
        return Err(invalid("Xaero block palette is too large"));
    }
    let (block, consumed) = decode(input.0)?;
    if consumed == 0 { return Err(invalid("Invalid Xaero block state")); }
    input.take(consumed)?;
    palette.push(block.clone());
    Ok(block)
}

fn utf(input: &mut Bytes<'_>) -> io::Result<Arc<str>> {
    let size = input.short()?;
    let value = std::str::from_utf8(input.take(size)?)
        .map_err(|_| invalid("Invalid Xaero biome identifier"))?;
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err(invalid("Invalid Xaero biome identifier"));
    }
    Ok(Arc::from(value))
}

fn tinted_blend(background: u32, top: u32, opacity: u32) -> u32 {
    let channel = |shift: u32| {
        ((((background >> shift) & 255) * (255 - opacity) + ((top >> shift) & 255) * opacity) / 255) << shift
    };
    channel(0) | channel(8) | channel(16)
}

/// Reads the block palette region from Xaero's `region.xaero` archive member.
pub fn block_region(
    bytes: &[u8],
    coordinate: [i32; 2],
    cave: Option<i32>,
    decode: impl Fn(&[u8]) -> io::Result<(SavedBlock, usize)>,
    color: impl Fn(&SavedBlock, Option<&str>, [i32; 2]) -> u32,
) -> io::Result<Region> {
    if coordinate.iter().any(|n| n.unsigned_abs() > 58_594) {
        return Err(invalid("Xaero block region is outside the world"));
    }
    let mut input = Bytes(bytes);
    if input.byte()? != 255 {
        return Err(invalid("Unversioned Xaero block regions are unsupported"));
    }
    let version = input.int()?;
    let (major, minor) = (version >> 16, version & 65535);
    if !(4..=8).contains(&major) || !(5..=8).contains(&minor) {
        return Err(invalid("Unsupported Xaero block region version"));
    }
    let grass = SavedBlock { name: Arc::from("minecraft:grass_block"), color: 0x7fb238, air: false };
    let water = SavedBlock { name: Arc::from("minecraft:water"), color: 0x3f76e4, air: false };
    let mut palette = Vec::new();
    let mut biomes = Vec::<Arc<str>>::new();
    let mut tiles = Vec::new();
    let mut layer = cave.map_or(Layer::Surface, Layer::Below);
    let mut stored_height = None;
    let mut seen = [false; 64];
    while !input.0.is_empty() {
        let [tx, tz] = coordinates(input.byte()?)?;
        if std::mem::replace(&mut seen[(tx * 8 + tz) as usize], true) {
            return Err(invalid("Duplicate Xaero block region tile group"));
        }
        for dx in 0..4 {
            for dz in 0..4 {
                let mut flags = input.int()?;
                if flags == -1 { continue; }
                let chunk = [coordinate[0] * 32 + tx * 4 + dx, coordinate[1] * 32 + tz * 4 + dz];
                let mut tile = Tile { pixels: vec![0; 1024], biomes: vec![None;256], ..Tile::default() };
                let mut overlays = Vec::new();
                for x in 0..16 {
                    for z in 0..16 {
                        if x != 0 || z != 0 { flags = input.int()?; }
                        let block = if flags & 1 == 0 { grass.clone() } else {
                            palette_block(&mut input, &mut palette, flags & (1 << 21) != 0, &decode)?
                        };
                        let height = if flags & 64 != 0 { i32::from(input.byte()?) } else {
                            let bits = ((flags >> 12) & 255) | (((flags >> 25) & 15) << 8);
                            (bits << 20) >> 20
                        };
                        let height = if flags & (1 << 24) != 0 { i32::from(input.byte()?) } else { height };
                        overlays.clear();
                        if flags & 2 != 0 {
                            for _ in 0..input.byte()? {
                                let overlay = input.int()?;
                                let block = if overlay & 1 == 0 { water.clone() } else {
                                    palette_block(&mut input, &mut palette, overlay & (1 << 10) != 0, &decode)?
                                };
                                if overlay & 4 != 0 { input.int()?; }
                                let opacity = if minor >= 8 {
                                    (overlay >> 11) & 15
                                } else if overlay & 8 != 0 {
                                    input.int()?
                                } else { 1 };
                                overlays.push((block, opacity.clamp(0,15) as u32));
                            }
                        }
                        let biome = if flags & (1 << 20) == 0 {
                            None
                        } else if flags & (1 << 22) != 0 {
                            let value = if flags & (1 << 23) != 0 {
                                Arc::from(format!("xaeroworldmap:legacy_biome_{}", input.int()?))
                            } else { utf(&mut input)? };
                            if biomes.len() >= 65_536 { return Err(invalid("Xaero biome palette is too large")); }
                            biomes.push(value.clone());
                            Some(value)
                        } else {
                            Some(biomes.get(input.int()? as usize).cloned()
                                .ok_or_else(|| invalid("Unknown Xaero biome palette index"))?)
                        };
                        let i = (z * 16 + x) as usize;
                        tile.heights[i] = height;
                        let at = [chunk[0] * 16 + x, chunk[1] * 16 + z];
                        let mut rgb = color(&block, biome.as_deref(), at);
                        for (block, opacity) in overlays.iter().rev() {
                            rgb = tinted_blend(rgb, color(block, biome.as_deref(), at), (opacity + 1) * 16 - 1);
                        }
                        if !block.air || !overlays.is_empty() {
                            tile.pixels[i*4..i*4+4].copy_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255]);
                        }
                        tile.biomes[i] = biome;
                    }
                }
                input.byte()?; // interpretation revision
                if minor >= 6 {
                    let cave_top = input.int()?;
                    if minor >= 7 { input.byte()?; }
                    if let Some(hint) = cave {
                        let next = cave_layer(Some(cave_top), hint)?;
                        if stored_height.is_some_and(|previous| previous != next) {
                            return Err(invalid("Xaero region contains mixed cave heights"));
                        }
                        stored_height = Some(next);
                        layer = next;
                    }
                }
                if tile.pixels.chunks_exact(4).any(|rgba| rgba[3] != 0) { tiles.push((chunk,tile)); }
            }
        }
    }
    Ok(Region { layer, tiles })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_region_restores_palette_colors_heights_and_biomes() {
        let mut bytes = vec![255];
        bytes.extend_from_slice(&0x0008_0008_i32.to_be_bytes());
        bytes.push(0x23);
        let inline = 1 | (1 << 20) | (1 << 21) | (1 << 22) | (64 << 12);
        bytes.extend_from_slice(&i32::to_be_bytes(inline));
        bytes.push(99); // opaque inline block token for the decoder
        let biome = b"minecraft:plains";
        bytes.extend_from_slice(&(biome.len() as u16).to_be_bytes());
        bytes.extend_from_slice(biome);
        bytes.extend_from_slice(&((1 | (1 << 20) | (65 << 12)) as i32).to_be_bytes());
        bytes.extend_from_slice(&[0; 8]); // block and biome palette references
        for _ in 2..256 {
            bytes.extend_from_slice(&(66_i32 << 12).to_be_bytes());
        }
        bytes.push(1);
        bytes.extend_from_slice(&48_i32.to_be_bytes());
        bytes.push(32);
        for _ in 0..15 { bytes.extend_from_slice(&(-1_i32).to_be_bytes()); }
        let decode = |data: &[u8]| {
            assert_eq!(data[0],99);
            Ok((SavedBlock { name: Arc::from("minecraft:stone"), color: 0x939393, air: false },1))
        };
        let color = |block: &SavedBlock, _: Option<&str>, _| block.color;
        let result = block_region(&bytes,[-2,3],Some(40),decode,color).unwrap();
        assert_eq!(result.layer,Layer::Below(48));
        assert_eq!(result.tiles.len(),1);
        let (at, tile) = &result.tiles[0];
        assert_eq!(*at,[-56,108]);
        assert_eq!(&tile.pixels[..4], &[0x93,0x93,0x93,255]);
        assert_eq!(tile.heights[16],65);
        assert_eq!(tile.biomes[16].as_deref(), Some("minecraft:plains"));
        assert!(block_region(&bytes[..bytes.len()-1],[-2,3],None,decode,color).is_err());
    }

    #[test]
    fn decodes_color_and_height_at_negative_region_coordinates() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FORMAT.to_be_bytes());
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&48_i32.to_be_bytes());
        bytes.extend_from_slice(&32_i32.to_be_bytes());
        bytes.push(255);
        bytes.extend_from_slice(&[0; 4]);
        bytes.push(0x23);
        bytes.extend_from_slice(&[11, 22, 33, 255]);
        bytes.resize(bytes.len() + 16380, 0);
        bytes.push(0);
        bytes.resize(bytes.len() + 8192, 0);
        bytes.extend_from_slice(&8177_u64.to_be_bytes());
        bytes.resize(bytes.len() + 8184, 0);
        bytes.extend_from_slice(&[0; 4]);
        bytes.push(255);
        let region = region(&bytes, [-2, 3], Some(48)).unwrap();
        assert_eq!(region.layer, Layer::Below(48));
        assert_eq!(region.tiles.len(), 1);
        assert_eq!(region.tiles[0].0, [-56, 108]);
        assert_eq!(&region.tiles[0].1.pixels[..4], &[11, 22, 33, 255]);
        assert_eq!(region.tiles[0].1.heights[0], -15);
        assert!(super::region(&bytes[..bytes.len() - 1], [-2, 3], None).is_err());
        let mut old = bytes.clone();
        old[..4].copy_from_slice(&((3_i32 << 16) | 22).to_be_bytes());
        old.drain(16..24);
        let older = super::region(&old, [-2,3], Some(71)).unwrap();
        assert_eq!(older.layer, Layer::Below(71));
        assert_eq!(older.tiles[0].1.pixels, region.tiles[0].1.pixels);
    }
}
