// SPDX-License-Identifier: GPL-3.0-only
use ::trailgrid_maps::runtime::Dimension;
use horizon_core::nbt::Limits;
use horizon_protocol::buf::Reader;
use std::collections::BTreeMap;

const MAX_STONES: usize = 4096;

pub struct Waystone {
    kind: String,
    pub name: String,
    pub at: [i32; 3],
    pub dimension: Dimension,
    pub discovered: bool,
    pub hidden: bool,
}

pub fn dimension(id: &str) -> Option<Dimension> {
    Some(match id {
        "minecraft:overworld" => Dimension::Overworld,
        "minecraft:the_nether" => Dimension::Nether,
        "minecraft:the_end" => Dimension::End,
        _ => return None,
    })
}

fn component(reader: &mut Reader<'_>) -> Option<String> {
    let limits = Limits { max_bytes: 8192, max_nodes: 512, max_depth: 24 };
    let tag = reader.nbt_within(limits).ok()?;
    Some(super::remote::remote_text(&tag).plain().chars()
        .filter(|c| !c.is_control()).take(80).collect())
}

fn waystone(reader: &mut Reader<'_>) -> Option<(u128, Waystone)> {
    let kind = reader.string(128).ok()?;
    let id = reader.uuid().ok()?;
    let dimension = dimension(&reader.string(128).ok()?)?;
    let pos = reader.i64().ok()?;
    let at = [(pos >> 38) as i32, (pos << 52 >> 52) as i32, (pos << 26 >> 38) as i32];
    let _origin = reader.varint().ok()?;
    let mut name = component(reader)?;
    let visibility = reader.varint().ok()?;
    if reader.bool().ok()? {
        let alias = component(reader)?;
        if !alias.is_empty() { name = alias; }
    }
    for _ in 0..reader.count(128).ok()? {
        reader.string(128).ok()?;
    }
    let hidden = reader.bool().ok()?;
    let discovered = kind == "waystones:waystone" && visibility != 3;
    Some((id, Waystone { kind, name, at, dimension, discovered, hidden }))
}

pub fn update_waystones(stones: &mut BTreeMap<u128, Waystone>, channel: &str, bytes: &[u8]) -> Option<()> {
    if bytes.len() > 1_048_576 {
        return None;
    }
    let mut reader = Reader::new(bytes);
    match channel {
        "waystones:known_waystones" => {
            let kind = reader.string(128).ok()?;
            let mut list = BTreeMap::new();
            for _ in 0..reader.count(MAX_STONES).ok()? {
                let (id, stone) = waystone(&mut reader)?;
                if stone.kind != kind { return None; }
                list.insert(id, stone);
            }
            if reader.remaining() != 0 { return None; }
            let retained = stones.values().filter(|stone| stone.kind != kind).count();
            if retained + list.len() > MAX_STONES { return None; }
            stones.retain(|_, stone| stone.kind != kind);
            stones.extend(list);
        }
        "waystones:update_waystone" => {
            let (id, stone) = waystone(&mut reader)?;
            if reader.remaining() != 0 || stones.len() >= MAX_STONES && !stones.contains_key(&id) {
                return None;
            }
            stones.insert(id, stone);
        }
        "waystones:waystone_removed" => {
            let kind = reader.string(128).ok()?;
            let id = reader.uuid().ok()?;
            let _destroyed = reader.bool().ok()?;
            if reader.remaining() != 0 { return None; }
            if stones.get(&id).is_some_and(|stone| stone.kind == kind) {
                stones.remove(&id);
            }
        }
        _ => return None,
    }
    Some(())
}

// OpenPAC announces a dimension, owner states and region palettes independently.
// Retain their indices so a removed state is immediately invisible in all regions.
use ::trailgrid_maps::map_claims::Chunk;
use horizon_core::nbt::Tag;
const MAX_CLAIM_CHUNKS: usize = 65_536;

#[derive(Default)]
pub struct OpenPac {
    dimension: Option<Dimension>,
    owners: BTreeMap<i32, u128>,
    names: BTreeMap<u128, String>,
    chunks: BTreeMap<Chunk, i32>,
    last: Option<(Chunk, Option<i32>)>,
}

fn uuid(tag: &Tag) -> Option<u128> {
    let Tag::IntArray(values) = tag.get("p")? else { return None; };
    let words: [i32; 4] = values.as_slice().try_into().ok()?;
    Some(words.into_iter().fold(0, |id, word| (id << 32) | u128::from(word as u32)))
}

fn int(tag: &Tag, key: &str) -> Option<i32> {
    match tag.get(key)? { Tag::Int(value) => Some(*value), _ => None }
}

fn tag_dimension(tag: &Tag) -> Option<Dimension> {
    match tag.get("d")? { Tag::String(value) => dimension(&value.to_string_lossy()), _ => None }
}

impl OpenPac {
    pub fn owner(&self, chunk: Chunk) -> Option<(u128, Option<&str>)> {
        let id = *self.owners.get(self.chunks.get(&chunk)?)?;
        Some((id, self.names.get(&id).map(String::as_str)))
    }

    pub fn chunks(&self, dimension: Dimension, min: [i32; 2], max: [i32; 2]) -> impl Iterator<Item = Chunk> + '_ {
        self.chunks.range((dimension,min[0],i32::MIN)..=(dimension,max[0],i32::MAX))
            .map(|(key, _)| *key).filter(move |(_,_,z)| (min[1]..=max[1]).contains(z))
    }

    fn set_chunk(&mut self, chunk: Chunk, state: Option<i32>) {
        if let Some(state) = state {
            if self.chunks.len() < MAX_CLAIM_CHUNKS || self.chunks.contains_key(&chunk) {
                self.chunks.insert(chunk, state);
            }
        } else {
            self.chunks.remove(&chunk);
        }
        self.last = Some((chunk, state));
    }

    pub fn update(&mut self, bytes: &[u8]) -> Option<()> {
        let (&id, bytes) = bytes.split_first()?;
        if bytes.len() > 65536 { return None; }
        match id {
            35 | 36 if bytes.is_empty() => {
                let ((d,x,z), claim) = self.last?;
                self.set_chunk((d,x.checked_add(i32::from(id==35))?,z.checked_add(i32::from(id==36))?),claim);
                return Some(());
            }
            48 if bytes.is_empty() => { *self = Self::default(); return Some(()); }
            10 | 12 | 13 | 14 | 26 | 27 | 29 => {}
            _ => return None,
        }
        let mut reader = Reader::new(bytes);
        let tag = reader.nbt_within(Limits { max_bytes: 65536, max_nodes: 8192, max_depth: 12 }).ok()?;
        if reader.remaining() != 0 || !matches!(tag, Tag::Compound(_)) { return None; }
        match id {
            10 => { self.dimension = tag_dimension(&tag); self.last = None; }
            12 | 26 => {
                let Tag::List(list) = tag.get("l")? else { return None; };
                if list.len() > 128 { return None; }
                for row in list {
                    let Some(player) = uuid(row) else { continue; };
                    if id == 12 && let Some(state) = int(row, "i") && self.owners.len() < 32768 {
                        self.owners.insert(state, player);
                    } else if id == 26 && let Some(Tag::String(name)) = row.get("u") && self.names.len() < 32768 {
                        self.names.insert(player, name.to_string_lossy().chars().filter(|c| !c.is_control()).take(64).collect());
                    }
                }
            }
            13 => {
                let dimension = self.dimension?;
                let x = int(&tag, "x")?.checked_mul(32)?;
                let z = int(&tag, "z")?.checked_mul(32)?;
                if x.abs_diff(0) > 1_875_040 || z.abs_diff(0) > 1_875_040 { return None; }
                let Tag::Byte(bits) = tag.get("b")? else { return None; };
                let Tag::IntArray(palette) = tag.get("p")? else { return None; };
                let Tag::LongArray(data) = tag.get("d")? else { return None; };
                let bits = usize::try_from(*bits).ok().filter(|bits| (1..=16).contains(bits))?;
                let per_word = 64 / bits;
                if palette.len() > 1024 || data.len() != 1024usize.div_ceil(per_word) { return None; }
                let mask = (1_u64 << bits) - 1;
                for i in 0..1024 {
                    let index = ((data[i / per_word] as u64 >> ((i % per_word) * bits)) & mask) as usize;
                    let key = (dimension, x + (i / 32) as i32, z + (i % 32) as i32);
                    if let Some(state) = index.checked_sub(1).and_then(|n| palette.get(n)) {
                        if self.chunks.len() < MAX_CLAIM_CHUNKS || self.chunks.contains_key(&key) {
                            self.chunks.insert(key, *state);
                        }
                    } else {
                        self.chunks.remove(&key);
                    }
                }
            }
            14 => {
                let key = (tag_dimension(&tag)?, int(&tag,"x")?, int(&tag,"z")?);
                let claim = if let Some(owner) = uuid(&tag) {
                    let state = int(&tag,"i")?;
                    if self.owners.len() < 32768 { self.owners.insert(state, owner); }
                    Some(state)
                } else { None };
                self.set_chunk(key, claim);
            }
            27 => { self.owners.remove(&int(&tag,"i")?); }
            29 => {
                let ((d,_,_), claim) = self.last?;
                self.set_chunk((d,int(&tag,"x")?,int(&tag,"z")?),claim);
            }
            _ => return None,
        }
        Some(())
    }
}
