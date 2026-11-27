// SPDX-License-Identifier: GPL-3.0-only
//! Shared explored ground: what either player explored shows on both maps.
//!
//! - **Up** (client → server): every surface chunk this client's map scans is queued
//!   ([`upload`], from the map worker) and sent a few per tick ([`client_tick`]), one payload a
//!   chunk: its 16×16 colours and heights ([`encode`], 1536 bytes).
//! - **Kept** (server): one [`Store`] per world, in `<world>/map-shared-ground.bin`, at most
//!   [`STORE_MAX`] chunks (the oldest go first). A chunk sent again replaces its copy.
//! - **Down** (server → client): each player gets, a few per tick, only the chunks of the
//!   dimension they are in that others explored and that they haven't been sent yet (a cursor
//!   per player and dimension, kept with the store): deltas, never the whole map again.
//! The world's shared waypoints (`points.rs`) live beside the ground in the same server half.
//! - **Shown** (client): a chunk from a friend fills ground this map has never seen; what this
//!   client saw itself is never overwritten ([`crate::runtime::maps::Cache::insert_shared`]).
use crate::kit::sync::lock;
use crate::runtime::Dimension;
use crate::runtime::maps::Tile;
use mod_api::{ClientHost, IPayloadContext, PayloadCodec, ServerHost, ToClient, ToServer, VarInt};
#[allow(unused_imports)]
use mod_api::prelude::*;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// A chunk's colours (16×16 RGBA) and surface heights (16×16 i16).
pub const TILE_BYTES: usize = 1024 + 512;
/// The most chunks a world keeps (~25 MB on disk): a 2048-block square of explored ground.
pub const STORE_MAX: usize = 16_384;
/// Chunks waiting to go up, at most; the oldest are dropped past it.
const OUTBOX_MAX: usize = 4096;
/// Chunks come down waiting for the map, at most.
const INBOX_MAX: usize = 4096;
/// Chunks sent up per client tick, and down per player per server tick.
const UP_PER_TICK: usize = 4;
const DOWN_PER_TICK: usize = 8;
/// Entries one player's delta looks through per tick, so a big store never stalls a tick.
const SCAN_PER_TICK: usize = 2048;
/// The store's file in the world folder, and its format's first bytes.
const FILE: &str = "map-shared-ground.bin";
const MAGIC: &[u8; 4] = b"MSG1";
/// No chunk lies beyond the world border (30 million blocks).
const CHUNK_LIMIT: i32 = 1_875_000;

/// One explored chunk, either way.
#[derive(PayloadCodec, Clone, Debug, PartialEq)]
pub struct Chunk {
    /// 0 overworld, 1 the Nether, 2 the End.
    pub dimension: VarInt,
    pub x: VarInt,
    pub z: VarInt,
    pub data: Vec<u8>,
}

pub static UP: OnceLock<ToServer<Chunk>> = OnceLock::new();
pub static DOWN: OnceLock<ToClient<Chunk>> = OnceLock::new();

/// The dimensions shared, by number.
pub fn code(dimension: Dimension) -> Option<u8> {
    match dimension {
        Dimension::Overworld => Some(0),
        Dimension::Nether => Some(1),
        Dimension::End => Some(2),
        Dimension::Player => None,
    }
}

pub fn dimension(code: u8) -> Option<Dimension> {
    [Dimension::Overworld, Dimension::Nether, Dimension::End].get(usize::from(code)).copied()
}

/// A level id's number (`minecraft:the_nether` → 1); none for a dimension a mod added.
pub fn code_of_id(id: &str) -> Option<u8> {
    match id {
        "minecraft:overworld" => Some(0),
        "minecraft:the_nether" => Some(1),
        "minecraft:the_end" => Some(2),
        _ => None,
    }
}

/// A surface tile as it travels; none for a tile that isn't a full chunk.
pub fn encode(tile: &Tile) -> Option<Vec<u8>> {
    if tile.pixels.len() != 1024 {
        return None;
    }
    let mut out = Vec::with_capacity(TILE_BYTES);
    out.extend_from_slice(&tile.pixels);
    for h in tile.heights {
        out.extend_from_slice(&(h.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16).to_le_bytes());
    }
    Some(out)
}

/// The tile back (no biomes, walls or portals: a friend's chunk shows its ground only).
pub fn decode(bytes: &[u8]) -> Option<Tile> {
    if bytes.len() != TILE_BYTES {
        return None;
    }
    let mut tile = Tile { pixels: bytes[..1024].to_vec(), ..Tile::default() };
    for (i, h) in bytes[1024..].chunks_exact(2).enumerate() {
        tile.heights[i] = i32::from(i16::from_le_bytes([h[0], h[1]]));
    }
    Some(tile)
}

type Key = (u8, i32, i32);

struct Entry {
    seq: u64,
    owner: u128,
    data: Vec<u8>,
}

/// A world's shared ground: every chunk with who sent it and when (a sequence number), and how
/// far each player's deltas have gone in each dimension.
#[derive(Default)]
pub struct Store {
    entries: BTreeMap<Key, Entry>,
    /// The same entries by sequence number, oldest first.
    order: BTreeMap<u64, Key>,
    seq: u64,
    cursors: BTreeMap<(u128, u8), u64>,
    pub dirty: bool,
}

impl Store {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Keeps `data` for the chunk `key` from `owner`. False when refused (not a chunk's
    /// bytes, outside the world, an unknown dimension) or unchanged.
    pub fn put(&mut self, key: Key, owner: u128, data: Vec<u8>) -> bool {
        let (d, x, z) = key;
        if data.len() != TILE_BYTES || dimension(d).is_none() || x.abs() > CHUNK_LIMIT || z.abs() > CHUNK_LIMIT {
            return false;
        }
        if self.entries.get(&key).is_some_and(|e| e.data == data) {
            return false;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.order.remove(&old.seq);
        }
        self.seq += 1;
        self.order.insert(self.seq, key);
        self.entries.insert(key, Entry { seq: self.seq, owner, data });
        while self.entries.len() > STORE_MAX {
            let Some((_, oldest)) = self.order.pop_first() else { break };
            self.entries.remove(&oldest);
        }
        self.dirty = true;
        true
    }

    /// What `player` should be sent next in dimension `d`: up to `limit` chunks others sent, in
    /// the order they came, after the player's cursor; and how far the look went (`scanned`),
    /// past chunks that aren't for them, so the cursor can move on when all were sent.
    pub fn pending(&self, player: u128, d: u8, limit: usize) -> (Vec<(Key, u64, &[u8])>, u64) {
        let from = self.cursors.get(&(player, d)).copied().unwrap_or(0);
        let mut out = Vec::new();
        let mut scanned = from;
        for (&seq, key) in self.order.range(from + 1..).take(SCAN_PER_TICK) {
            let entry = &self.entries[key];
            if key.0 == d && entry.owner != player {
                if out.len() == limit {
                    break;
                }
                out.push((*key, seq, entry.data.as_slice()));
            }
            scanned = seq;
        }
        (out, scanned)
    }

    /// `player`'s deltas in dimension `d` went as far as `seq`.
    pub fn sent(&mut self, player: u128, d: u8, seq: u64) {
        let cursor = self.cursors.entry((player, d)).or_insert(0);
        if seq > *cursor {
            *cursor = seq;
            self.dirty = true;
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.entries.len() * (33 + TILE_BYTES));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (&(d, x, z), e) in &self.entries {
            out.push(d);
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&z.to_le_bytes());
            out.extend_from_slice(&e.seq.to_le_bytes());
            out.extend_from_slice(&e.owner.to_le_bytes());
            out.extend_from_slice(&e.data);
        }
        out.extend_from_slice(&(self.cursors.len() as u32).to_le_bytes());
        for (&(player, d), &seq) in &self.cursors {
            out.extend_from_slice(&player.to_le_bytes());
            out.push(d);
            out.extend_from_slice(&seq.to_le_bytes());
        }
        out
    }

    /// A store read back; none for bytes that aren't one (a damaged file is kept aside).
    pub fn from_bytes(bytes: &[u8]) -> Option<Store> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC {
            return None;
        }
        let mut store = Store { seq: r.u64()?, ..Store::default() };
        for _ in 0..r.u32()? {
            let key = (r.take(1)?[0], r.i32()?, r.i32()?);
            let (seq, owner) = (r.u64()?, r.u128()?);
            let data = r.take(TILE_BYTES)?.to_vec();
            store.order.insert(seq, key);
            store.entries.insert(key, Entry { seq, owner, data });
        }
        for _ in 0..r.u32()? {
            let (player, d, seq) = (r.u128()?, r.take(1)?[0], r.u64()?);
            store.cursors.insert((player, d), seq);
        }
        (r.0.is_empty() && store.order.len() == store.entries.len()).then_some(store)
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.take(16)?.try_into().ok()?))
    }
}

// ---- Client ----

/// Chunks waiting to go up: the latest copy of each, oldest first.
#[derive(Default)]
struct Outbox {
    chunks: BTreeMap<Key, Vec<u8>>,
    order: VecDeque<Key>,
}

impl Outbox {
    fn push(&mut self, key: Key, data: Vec<u8>) {
        if self.chunks.insert(key, data).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > OUTBOX_MAX {
            if let Some(old) = self.order.pop_front() {
                self.chunks.remove(&old);
            }
        }
    }
    fn front(&self) -> Option<(Key, &Vec<u8>)> {
        let key = *self.order.front()?;
        Some((key, &self.chunks[&key]))
    }
    fn pop(&mut self) {
        if let Some(key) = self.order.pop_front() {
            self.chunks.remove(&key);
        }
    }
}

static OUTBOX: Mutex<Option<Outbox>> = Mutex::new(None);
static INBOX: Mutex<VecDeque<(Key, Vec<u8>)>> = Mutex::new(VecDeque::new());

/// The map worker scanned a surface chunk of this client's own: it goes up.
pub fn upload(dimension: Dimension, chunk: [i32; 2], tile: &Tile) {
    let (Some(d), Some(data)) = (code(dimension), encode(tile)) else { return };
    lock(&OUTBOX).get_or_insert_with(Outbox::default).push((d, chunk[0], chunk[1]), data);
}

/// Client tick: a few waiting chunks go up (a send refused, as early after joining or on a server
/// without Map, keeps them for later).
pub fn client_tick(host: &mut dyn ClientHost) {
    let Some(token) = UP.get() else { return };
    let mut outbox = lock(&OUTBOX);
    let Some(outbox) = outbox.as_mut() else { return };
    for _ in 0..UP_PER_TICK {
        let Some(((d, x, z), data)) = outbox.front() else { return };
        let chunk = Chunk { dimension: VarInt(i32::from(d)), x: VarInt(x), z: VarInt(z), data: data.clone() };
        if token.send(host, &chunk).is_err() {
            return;
        }
        outbox.pop();
    }
}

/// A friend's chunk came down: kept until the map takes it.
pub fn client_receive(chunk: Chunk, _: &mut dyn IPayloadContext) -> Result<(), String> {
    let Ok(d) = u8::try_from(chunk.dimension.0) else { return Ok(()) };
    if chunk.data.len() == TILE_BYTES && dimension(d).is_some() {
        let mut inbox = lock(&INBOX);
        inbox.push_back(((d, chunk.x.0, chunk.z.0), chunk.data));
        while inbox.len() > INBOX_MAX {
            inbox.pop_front();
        }
    }
    Ok(())
}

/// Hands the friends' chunks of `current` to `submit` (the map's worker) while it takes them;
/// the rest wait, other dimensions' are dropped (the server sends them again there).
pub fn drain_incoming(current: Dimension, mut submit: impl FnMut([i32; 2], Tile) -> bool) {
    let mut inbox = lock(&INBOX);
    while let Some(((d, x, z), data)) = inbox.front() {
        let (here, chunk, tile) = (dimension(*d) == Some(current), [*x, *z], decode(data));
        if let (true, Some(tile)) = (here, tile)
            && !submit(chunk, tile)
        {
            return;
        }
        inbox.pop_front();
    }
}

/// Leaving a world: nothing waits for the next one.
pub fn client_reset() {
    *lock(&OUTBOX) = None;
    lock(&INBOX).clear();
}

// ---- Server ----

#[derive(Default)]
struct Server {
    /// The world folder the store belongs to.
    world: Option<PathBuf>,
    store: Store,
    /// The world's shared waypoints (`points.rs`), kept and saved beside the ground.
    points: crate::points::Points,
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);
/// The shared waypoints' file in the world folder.
const POINTS_FILE: &str = "map-shared-points.txt";

/// The loaded world's shared waypoints, when a world is loaded.
pub fn with_points(f: impl FnOnce(&mut crate::points::Points)) {
    if let Some(server) = lock(&SERVER).as_mut()
        && server.world.is_some()
    {
        f(&mut server.points);
    }
}

/// Server tick: the shared waypoints' changes go down too.
pub fn points_tick(host: &mut dyn ServerHost) {
    if let Some(server) = lock(&SERVER).as_mut() {
        crate::points::server_tick(host, &mut server.points);
    }
}

fn file(world: &Path) -> PathBuf {
    world.join(FILE)
}

fn read(world: &Path) -> Store {
    let path = file(world);
    let Ok(bytes) = std::fs::read(&path) else { return Store::default() };
    Store::from_bytes(&bytes).unwrap_or_else(|| {
        eprintln!("[map] {} isn't a shared-ground file; kept aside as .broken", path.display());
        let _ = std::fs::rename(&path, path.with_extension("bin.broken"));
        Store::default()
    })
}

fn write(world: &Path, store: &Store) -> std::io::Result<()> {
    let path = file(world);
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, store.to_bytes())?;
    std::fs::rename(&tmp, &path)
}

/// A player's file was read from `<world>/players/data`: the world's stores are loaded once, and
/// the joining `player` is owed every shared waypoint again.
pub fn world(directory: &Path, player: u128) {
    let Some(world) = directory.parent().and_then(Path::parent) else { return };
    let mut server = lock(&SERVER);
    let server = server.get_or_insert_with(Server::default);
    if server.world.as_deref() != Some(world) {
        save_into(server);
        let points = std::fs::read_to_string(world.join(POINTS_FILE)).map(|t| crate::points::Points::from_text(&t)).unwrap_or_default();
        *server = Server { world: Some(world.to_path_buf()), store: read(world), points };
    }
    server.points.joined(player);
}

fn save_into(server: &mut Server) {
    let Some(world) = &server.world else { return };
    if server.store.dirty {
        match write(world, &server.store) {
            Ok(()) => server.store.dirty = false,
            Err(e) => eprintln!("[map] saving the shared ground: {e}"),
        }
    }
    if server.points.dirty {
        let path = world.join(POINTS_FILE);
        let tmp = path.with_extension("txt.tmp");
        match std::fs::write(&tmp, server.points.to_text()).and_then(|()| std::fs::rename(&tmp, &path)) {
            Ok(()) => server.points.dirty = false,
            Err(e) => eprintln!("[map] saving the shared waypoints: {e}"),
        }
    }
}

/// The world saved: so does the shared ground, when it changed.
pub fn save() {
    if let Some(server) = lock(&SERVER).as_mut() {
        save_into(server);
    }
}

/// The server stops: saved, and forgotten for the next world.
pub fn stop() {
    if let Some(mut server) = lock(&SERVER).take() {
        save_into(&mut server);
    }
}

/// A chunk came up from a player.
pub fn server_receive(chunk: Chunk, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let player = cx.player().map_err(|e| format!("{e:?}"))?;
    let Ok(d) = u8::try_from(chunk.dimension.0) else { return Ok(()) };
    if let Some(server) = lock(&SERVER).as_mut()
        && server.world.is_some()
    {
        server.store.put((d, chunk.x.0, chunk.z.0), player.uuid(), chunk.data);
    }
    Ok(())
}

/// Server tick: each player gets a few chunks others explored in the dimension they are in.
pub fn server_tick(host: &mut dyn ServerHost) {
    let Some(token) = DOWN.get() else { return };
    let players: Vec<_> = host.players().server_players();
    let mut server = lock(&SERVER);
    let Some(server) = server.as_mut() else { return };
    for p in players {
        let Some(d) = p.dimension(&*host).and_then(|id| code_of_id(&id.to_string())) else { continue };
        let (chunks, scanned) = server.store.pending(p.uuid(), d, DOWN_PER_TICK);
        let mut reached = scanned;
        let mut sends = Vec::new();
        for ((d, x, z), seq, data) in chunks {
            sends.push((seq, Chunk { dimension: VarInt(i32::from(d)), x: VarInt(x), z: VarInt(z), data: data.to_vec() }));
        }
        for (seq, chunk) in sends {
            if token.send(host, p, &chunk).is_err() {
                reached = seq - 1;
                break;
            }
        }
        server.store.sent(p.uuid(), d, reached);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(seed: u8) -> Vec<u8> {
        (0..TILE_BYTES).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    #[test]
    fn a_tile_travels_whole() {
        let mut tile = Tile { pixels: (0..1024).map(|i| i as u8).collect(), ..Tile::default() };
        tile.heights[0] = -64;
        tile.heights[255] = 319;
        tile.heights[7] = 99_999;
        let bytes = encode(&tile).unwrap();
        assert_eq!(bytes.len(), TILE_BYTES);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.pixels, tile.pixels);
        assert_eq!((back.heights[0], back.heights[255], back.heights[7]), (-64, 319, i32::from(i16::MAX)));
        assert!(encode(&Tile::default()).is_none(), "an empty tile isn't sent");
        assert!(decode(&bytes[1..]).is_none());
    }

    #[test]
    fn each_player_gets_only_what_others_explored_in_their_dimension_once() {
        let (a, b) = (1u128, 2u128);
        let mut s = Store::default();
        assert!(s.put((0, 0, 0), a, data(1)));
        assert!(s.put((0, 1, 0), b, data(2)));
        assert!(s.put((1, 5, 5), a, data(3)));
        // B in the overworld: A's overworld chunk only.
        let (chunks, scanned) = s.pending(b, 0, 8);
        assert_eq!(chunks.iter().map(|c| c.0).collect::<Vec<_>>(), [(0, 0, 0)]);
        s.sent(b, 0, scanned);
        assert!(s.pending(b, 0, 8).0.is_empty(), "sent once: a delta, not the map again");
        // B goes to the Nether: A's Nether chunk, which the overworld cursor never passed over.
        assert_eq!(s.pending(b, 1, 8).0.iter().map(|c| c.0).collect::<Vec<_>>(), [(1, 5, 5)]);
        // A gets B's chunk, never their own.
        assert_eq!(s.pending(a, 0, 8).0.iter().map(|c| c.0).collect::<Vec<_>>(), [(0, 1, 0)]);
        // A explores the same chunk again with something new: B is sent it again.
        assert!(s.put((0, 0, 0), a, data(9)));
        assert_eq!(s.pending(b, 0, 8).0.len(), 1);
        assert!(!s.put((0, 0, 0), a, data(9)), "unchanged: nothing to send");
    }

    #[test]
    fn deltas_come_a_few_at_a_time_and_a_failed_send_is_tried_again() {
        let mut s = Store::default();
        for x in 0..20 {
            s.put((0, x, 0), 1, data(x as u8));
        }
        let (chunks, _) = s.pending(2, 0, 8);
        assert_eq!(chunks.len(), 8);
        let third = chunks[2].1;
        // Two went, the third failed: the next tick starts at the third.
        s.sent(2, 0, third - 1);
        assert_eq!(s.pending(2, 0, 8).0[0].0, (0, 2, 0));
    }

    #[test]
    fn the_store_is_capped_oldest_first_and_refuses_bad_chunks() {
        let mut s = Store::default();
        for i in 0..(STORE_MAX as i32 + 5) {
            s.put((0, i, 0), 1, data(0));
        }
        assert_eq!(s.len(), STORE_MAX);
        assert!(s.entries.contains_key(&(0, STORE_MAX as i32 + 4, 0)));
        assert!(!s.entries.contains_key(&(0, 4, 0)), "the oldest went first");
        assert!(!s.put((0, 0, 0), 1, vec![0; 10]), "not a chunk's bytes");
        assert!(!s.put((7, 0, 0), 1, data(0)), "no such dimension");
        assert!(!s.put((0, CHUNK_LIMIT + 1, 0), 1, data(0)), "beyond the border");
    }

    #[test]
    fn the_store_survives_a_restart() {
        let mut s = Store::default();
        s.put((0, -3, 7), 11, data(4));
        s.put((2, 100, -100), 12, data(5));
        s.sent(12, 0, 1);
        let back = Store::from_bytes(&s.to_bytes()).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.seq, s.seq);
        assert_eq!(back.entries[&(0, -3, 7)].data, data(4));
        assert_eq!(back.entries[&(2, 100, -100)].owner, 12);
        assert!(back.pending(12, 0, 8).0.is_empty(), "the cursor came back too");
        let bytes = s.to_bytes();
        assert!(Store::from_bytes(&bytes[..bytes.len() - 1]).is_none(), "a cut file is refused");
        assert!(Store::from_bytes(b"nope").is_none());
    }

    #[test]
    fn the_outbox_keeps_the_latest_copy_once_and_is_capped() {
        let mut o = Outbox::default();
        o.push((0, 1, 1), data(1));
        o.push((0, 2, 2), data(2));
        o.push((0, 1, 1), data(3));
        assert_eq!(o.order.len(), 2);
        assert_eq!(o.front().unwrap(), ((0, 1, 1), &data(3)));
        for i in 0..(OUTBOX_MAX as i32 + 10) {
            o.push((1, i, 0), data(0));
        }
        assert_eq!(o.order.len(), OUTBOX_MAX);
        assert_eq!(o.chunks.len(), OUTBOX_MAX);
    }

    #[test]
    fn dimensions_have_one_number_each_way() {
        for d in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
            assert_eq!(dimension(code(d).unwrap()), Some(d));
            assert_eq!(code_of_id(d.id()), code(d));
        }
        assert_eq!(code(Dimension::Player), None);
        assert_eq!(code_of_id("acme:moon"), None);
    }

    #[test]
    fn a_friends_chunk_never_covers_what_this_map_saw_itself() {
        use crate::runtime::maps::{Cache, Layer};
        let mut cache = Cache::default();
        let own = Tile { pixels: vec![1; 1024], ..Tile::default() };
        cache.insert(Dimension::Overworld, Layer::Surface, [0, 0], own);
        let friend = Tile { pixels: vec![2; 1024], ..Tile::default() };
        assert!(!cache.insert_shared(Dimension::Overworld, [0, 0], friend.clone()));
        assert!(cache.insert_shared(Dimension::Overworld, [1, 0], friend));
        assert_eq!(cache.surface_pixels(Dimension::Overworld, [0, 0]).unwrap()[0], 1);
        assert_eq!(cache.surface_pixels(Dimension::Overworld, [1, 0]).unwrap()[0], 2);
    }
}
