// SPDX-License-Identifier: GPL-3.0-only
//! Shared waypoints: a waypoint a player makes, and their death points, show on the other players'
//! minimap and world map in the maker's colour, without anyone pressing Share.
//!
//! The same sharing as the Share button, carried for you: each waypoint travels as its share link
//! ([`crate::runtime::Waypoint::share_link`], read back by [`crate::runtime::receive_shared_waypoint`]),
//! marked `d|` for a death point or `w|` for any other.
//!
//! - **Up** (client → server): the list of this player's shared waypoints (every enabled one whose
//!   set is shared; none when the setting `share_waypoints` is off), sent again whenever it changes
//!   ([`offer`], [`client_tick`]).
//! - **Kept** (server): [`Points`], per world beside the shared ground (`map-shared-points.txt`), at
//!   most [`OWN_MAX`] a player. A list replaces the player's own entries only: nobody edits another
//!   player's waypoints. A waypoint gone from the list becomes a removal.
//! - **Down** (server → client): each player gets only the changes since their cursor, a few per
//!   tick; a player who joins starts from nothing and gets every waypoint again (a client keeps
//!   friends' waypoints for the session only).
use crate::kit::sync::lock;
use crate::runtime::{Dimension, Waypoint, WaypointIcon, receive_shared_waypoint};
use mod_api::{ClientHost, IPayloadContext, PayloadCodec, ServerHost, ToClient, ToServer};
#[allow(unused_imports)]
use mod_api::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

/// The most waypoints one player shares.
pub const OWN_MAX: usize = 256;
/// The most entries the server keeps, removals included (the oldest removals go first).
pub const STORE_MAX: usize = 4096;
/// The longest waypoint text accepted.
const TEXT_MAX: usize = 512;
/// Changes sent down per player per server tick.
const DOWN_PER_TICK: usize = 16;
/// Entries one player's delta looks through per tick.
const SCAN_PER_TICK: usize = 1024;
/// The world a friend's waypoint is read into (it is drawn, never saved).
const SHARED_WORLD: &str = "server:shared";

/// Client → server: every waypoint this player shares now.
#[derive(PayloadCodec, Clone, Debug, PartialEq)]
pub struct List {
    pub points: Vec<String>,
}

/// Server → client: one waypoint of another player, added or removed.
#[derive(PayloadCodec, Clone, Debug, PartialEq)]
pub struct Change {
    pub owner: String,
    pub alive: bool,
    pub point: String,
}

pub static UP: OnceLock<ToServer<List>> = OnceLock::new();
pub static DOWN: OnceLock<ToClient<Change>> = OnceLock::new();

/// A waypoint as it travels.
pub fn encode(point: &Waypoint) -> String {
    let kind = if point.icon == WaypointIcon::Skull { 'd' } else { 'w' };
    format!("{kind}|{}", point.share_link())
}

/// The waypoint back; none for text that isn't one.
pub fn decode(text: &str) -> Option<Waypoint> {
    if text.len() > TEXT_MAX {
        return None;
    }
    let (kind, link) = text.split_once('|')?;
    let mut point = receive_shared_waypoint(link, SHARED_WORLD, Dimension::Overworld)?;
    point.icon = match kind {
        "d" => WaypointIcon::Skull,
        "w" => WaypointIcon::Letter,
        _ => return None,
    };
    Some(point)
}

// ---- Server ----

struct Entry {
    seq: u64,
    owner: String,
    alive: bool,
}

type Key = (u128, String);

/// A world's shared waypoints: each player's, with a sequence number each change takes, and how
/// far each online player has been sent.
#[derive(Default)]
pub struct Points {
    entries: BTreeMap<Key, Entry>,
    order: BTreeMap<u64, Key>,
    seq: u64,
    cursors: BTreeMap<u128, u64>,
    pub dirty: bool,
}

impl Points {
    fn touch(&mut self, key: Key, owner: &str, alive: bool) {
        if let Some(old) = self.entries.remove(&key) {
            self.order.remove(&old.seq);
        }
        self.seq += 1;
        self.order.insert(self.seq, key.clone());
        self.entries.insert(key, Entry { seq: self.seq, owner: owner.to_owned(), alive });
        self.dirty = true;
    }

    /// `player` (named `name`) now shares exactly `points`: new ones are added, ones gone are
    /// removed, the others stay as they are. Texts that aren't waypoints are left out. How many
    /// changed.
    pub fn replace(&mut self, player: u128, name: &str, points: Vec<String>) -> usize {
        let mut wanted: Vec<String> = points.into_iter().filter(|t| decode(t).is_some()).collect();
        wanted.sort();
        wanted.dedup();
        wanted.truncate(OWN_MAX);
        let had: Vec<String> = self.entries.iter().filter(|((p, _), e)| *p == player && e.alive).map(|((_, t), _)| t.clone()).collect();
        let mut changed = 0;
        for text in had.iter().filter(|t| wanted.binary_search(t).is_err()) {
            self.touch((player, text.clone()), name, false);
            changed += 1;
        }
        for text in wanted.iter().filter(|t| !had.contains(t)) {
            self.touch((player, text.clone()), name, true);
            changed += 1;
        }
        while self.entries.len() > STORE_MAX {
            let oldest = self.order.iter().find(|(_, k)| !self.entries[*k].alive).or_else(|| self.order.iter().next());
            let Some((&seq, key)) = oldest else { break };
            let key = key.clone();
            self.order.remove(&seq);
            self.entries.remove(&key);
        }
        changed
    }

    /// A player joined: they have none of the others' waypoints yet.
    pub fn joined(&mut self, player: u128) {
        self.cursors.remove(&player);
    }

    /// The changes `player` should get next (others' waypoints only; a player starting from
    /// nothing gets no removals), and how far the look went.
    pub fn pending(&self, player: u128, limit: usize) -> (Vec<(u64, Change)>, u64) {
        let from = self.cursors.get(&player).copied().unwrap_or(0);
        let mut out = Vec::new();
        let mut scanned = from;
        for (&seq, key) in self.order.range(from + 1..).take(SCAN_PER_TICK) {
            let entry = &self.entries[key];
            if key.0 != player && (entry.alive || from > 0) {
                if out.len() == limit {
                    break;
                }
                out.push((seq, Change { owner: entry.owner.clone(), alive: entry.alive, point: key.1.clone() }));
            }
            scanned = seq;
        }
        (out, scanned)
    }

    /// `player` was sent the changes up to `seq`.
    pub fn sent(&mut self, player: u128, seq: u64) {
        let cursor = self.cursors.entry(player).or_insert(0);
        *cursor = (*cursor).max(seq);
    }

    /// The live waypoints as kept on disk: `<uuid hex>\t<owner>\t<waypoint>` a line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for ((player, text), e) in self.entries.iter().filter(|(_, e)| e.alive) {
            out.push_str(&format!("{player:032x}\t{}\t{text}\n", e.owner));
        }
        out
    }

    pub fn from_text(text: &str) -> Points {
        let mut points = Points::default();
        let mut by_player: BTreeMap<u128, (String, Vec<String>)> = BTreeMap::new();
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(uuid), Some(owner), Some(point)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let Ok(uuid) = u128::from_str_radix(uuid, 16) else { continue };
            let entry = by_player.entry(uuid).or_insert_with(|| (owner.to_owned(), Vec::new()));
            entry.1.push(point.to_owned());
        }
        for (uuid, (owner, list)) in by_player {
            points.replace(uuid, &owner, list);
        }
        points.dirty = false;
        points
    }
}

/// Server tick: each player gets a few changes to the others' waypoints.
pub fn server_tick(host: &mut dyn ServerHost, points: &mut Points) {
    let Some(token) = DOWN.get() else { return };
    for p in host.players().server_players() {
        let (changes, scanned) = points.pending(p.uuid(), DOWN_PER_TICK);
        let mut reached = scanned;
        for (seq, change) in changes {
            if token.send(host, p, &change).is_err() {
                reached = seq - 1;
                break;
            }
        }
        points.sent(p.uuid(), reached);
    }
}

/// A player's list came up.
pub fn server_receive(list: List, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let player = cx.player().map_err(|e| format!("{e:?}"))?;
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    let name = player.name(&*host).unwrap_or_default();
    let points: Vec<String> = list.points.into_iter().take(OWN_MAX * 2).collect();
    crate::ground::with_points(|store| {
        store.replace(player.uuid(), &name, points);
    });
    Ok(())
}

// ---- Client ----

#[derive(Default)]
struct Client {
    /// The list last sent, and the one waiting to go.
    sent: Option<Vec<String>>,
    waiting: Option<Vec<String>>,
    /// What `offer` was last given, to skip work when nothing changed.
    offered: Option<(bool, Vec<Waypoint>)>,
    /// Other players' waypoints by their text: owner and the point.
    friends: BTreeMap<String, (String, Waypoint)>,
}

static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

fn client<R>(f: impl FnOnce(&mut Client) -> R) -> R {
    f(lock(&CLIENT).get_or_insert_with(Client::default))
}

/// The waypoints this player shares now (`sharing` off: none). Called every frame; work only
/// when they changed.
pub fn offer<'a>(sharing: bool, points: impl Iterator<Item = &'a Waypoint>) {
    let points: Vec<Waypoint> = if sharing { points.take(OWN_MAX).cloned().collect() } else { Vec::new() };
    client(|c| {
        if c.offered.as_ref().is_some_and(|(s, p)| *s == sharing && *p == points) {
            return;
        }
        let mut texts: Vec<String> = points.iter().map(encode).collect();
        texts.sort();
        texts.dedup();
        if c.sent.as_ref() != Some(&texts) {
            c.waiting = Some(texts);
        }
        c.offered = Some((sharing, points));
    });
}

/// Client tick: a changed list goes up (a refused send, as early after joining, is tried again).
pub fn client_tick(host: &mut dyn ClientHost) {
    let Some(token) = UP.get() else { return };
    let waiting = client(|c| c.waiting.clone());
    if let Some(points) = waiting
        && token.send(host, &List { points: points.clone() }).is_ok()
    {
        client(|c| {
            c.sent = Some(points);
            c.waiting = None;
        });
    }
}

/// Another player's waypoint came or went.
pub fn client_receive(change: Change, _: &mut dyn IPayloadContext) -> Result<(), String> {
    client(|c| apply(&mut c.friends, change));
    Ok(())
}

fn apply(friends: &mut BTreeMap<String, (String, Waypoint)>, change: Change) {
    if !change.alive {
        friends.remove(&change.point);
    } else if let Some(point) = decode(&change.point)
        && (friends.len() < STORE_MAX || friends.contains_key(&change.point))
    {
        friends.insert(change.point, (change.owner, point));
    }
}

/// Other players' waypoints in `dimension`, each in its maker's colour (`colour(owner)`), with the
/// maker's name.
pub fn friends(dimension: Dimension, colour: impl Fn(&str) -> u32) -> Vec<(String, Waypoint)> {
    client(|c| {
        c.friends
            .values()
            .filter(|(_, p)| p.dimension == dimension)
            .map(|(owner, p)| (owner.clone(), Waypoint { color: colour(owner) & 0x00ff_ffff, ..p.clone() }))
            .collect()
    })
}

/// Leaving a world: the next one starts empty, and our list is sent again there.
pub fn client_reset() {
    *lock(&CLIENT) = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::WaypointOptions;

    /// The client state is one static: its tests take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn point(name: &str, x: i32, death: bool) -> Waypoint {
        Waypoint {
            world: "server:test".into(),
            dimension: Dimension::Overworld,
            x,
            y: 64,
            z: -7,
            name: name.into(),
            initials: Some(name.chars().take(1).collect()),
            options: WaypointOptions::default(),
            color: 0xff0000,
            icon: if death { WaypointIcon::Skull } else { WaypointIcon::Letter },
            biome: None,
            show_3d: true,
        }
    }

    fn texts(names: &[(&str, i32)]) -> Vec<String> {
        names.iter().map(|(n, x)| encode(&point(n, *x, false))).collect()
    }

    #[test]
    fn a_waypoint_travels_as_its_share_link() {
        let p = point("Base: north", 120, false);
        let back = decode(&encode(&p)).unwrap();
        assert_eq!((back.name.as_str(), back.x, back.y, back.z, back.dimension), ("Base: north", 120, 64, -7, Dimension::Overworld));
        assert_eq!(back.icon, WaypointIcon::Letter);
        assert_eq!(decode(&encode(&point("Death", 5, true))).unwrap().icon, WaypointIcon::Skull);
        assert!(decode("x|nope").is_none() && decode("w|nope").is_none() && decode("nothing").is_none());
        let mut nether = p.clone();
        nether.dimension = Dimension::Nether;
        assert_eq!(decode(&encode(&nether)).unwrap().dimension, Dimension::Nether);
    }

    #[test]
    fn a_list_replaces_only_its_makers_waypoints_and_deltas_go_to_the_others() {
        let (a, b) = (1u128, 2u128);
        let mut s = Points::default();
        assert_eq!(s.replace(a, "A", texts(&[("Home", 0), ("Mine", 10)])), 2);
        assert_eq!(s.replace(b, "B", texts(&[("Farm", 50)])), 1);
        // B gets A's two, A gets B's one; nobody their own.
        let (to_b, reached) = s.pending(b, 16);
        assert_eq!(to_b.iter().map(|(_, c)| (c.owner.as_str(), c.alive)).collect::<Vec<_>>(), [("A", true), ("A", true)]);
        s.sent(b, reached);
        assert_eq!(s.pending(a, 16).0.len(), 1);
        // A moves the mine (an edit) and drops home: B is told of the three changes only.
        assert_eq!(s.replace(a, "A", texts(&[("Mine", 11)])), 3);
        let changes: Vec<_> = s.pending(b, 16).0.into_iter().map(|(_, c)| (decode(&c.point).unwrap().name, decode(&c.point).unwrap().x, c.alive)).collect();
        assert_eq!(changes.len(), 3);
        assert!(changes.contains(&("Home".into(), 0, false)) && changes.contains(&("Mine".into(), 10, false)) && changes.contains(&("Mine".into(), 11, true)));
        // Sending the same list again changes nothing.
        assert_eq!(s.replace(a, "A", texts(&[("Mine", 11)])), 0);
        // B's list never touches A's waypoints.
        s.replace(b, "B", Vec::new());
        assert_eq!(s.entries.iter().filter(|((p, _), e)| *p == a && e.alive).count(), 1);
    }

    #[test]
    fn a_player_who_joins_gets_every_live_waypoint_and_no_removals() {
        let mut s = Points::default();
        s.replace(1, "A", texts(&[("Old", 0), ("Keep", 1)]));
        s.replace(1, "A", texts(&[("Keep", 1)]));
        let (all, reached) = s.pending(2, 16);
        assert_eq!(all.len(), 1, "only the live one");
        s.sent(2, reached);
        s.joined(2);
        assert_eq!(s.pending(2, 16).0.len(), 1, "a rejoin sends it again");
    }

    #[test]
    fn caps_and_bad_texts() {
        let mut s = Points::default();
        let many: Vec<String> = (0..(OWN_MAX as i32 + 20)).map(|x| encode(&point("P", x, false))).collect();
        s.replace(1, "A", many);
        assert_eq!(s.entries.len(), OWN_MAX);
        assert_eq!(s.replace(2, "B", vec!["w|junk".into(), "x".repeat(600)]), 0);
        for player in 3..(3 + (STORE_MAX / OWN_MAX) as u128 + 2) {
            s.replace(player, "C", (0..OWN_MAX as i32).map(|x| encode(&point("Q", x, false))).collect());
        }
        assert!(s.entries.len() <= STORE_MAX);
    }

    #[test]
    fn the_store_survives_a_restart_without_its_removals() {
        let mut s = Points::default();
        s.replace(0xabc, "A", texts(&[("Home", 0), ("Gone", 3)]));
        s.replace(0xabc, "A", texts(&[("Home", 0)]));
        let back = Points::from_text(&s.to_text());
        assert_eq!(back.entries.len(), 1);
        assert!(!back.dirty);
        let (all, _) = back.pending(7, 16);
        assert_eq!((all[0].1.owner.as_str(), decode(&all[0].1.point).unwrap().name.as_str()), ("A", "Home"));
    }

    #[test]
    fn friends_waypoints_come_and_go_and_take_the_makers_colour() {
        let _serial = lock(&SERIAL);
        let mut friends = BTreeMap::new();
        let text = encode(&point("Cave", 4, false));
        apply(&mut friends, Change { owner: "P1".into(), alive: true, point: text.clone() });
        apply(&mut friends, Change { owner: "P1".into(), alive: true, point: "w|junk".into() });
        assert_eq!(friends.len(), 1);
        *lock(&CLIENT) = Some(Client { friends, ..Client::default() });
        let shown = super::friends(Dimension::Overworld, |owner| if owner == "P1" { 0xff12_3456 } else { 0 });
        assert_eq!((shown[0].0.as_str(), shown[0].1.color), ("P1", 0x12_3456));
        assert!(super::friends(Dimension::Nether, |_| 0).is_empty());
        client(|c| apply(&mut c.friends, Change { owner: "P1".into(), alive: false, point: text }));
        assert!(super::friends(Dimension::Overworld, |_| 0).is_empty());
        client_reset();
    }

    #[test]
    fn a_list_goes_up_only_when_it_changed_or_sharing_turned_off() {
        let _serial = lock(&SERIAL);
        client_reset();
        let points = [point("Home", 0, false)];
        offer(true, points.iter());
        let first = client(|c| c.waiting.clone()).unwrap();
        assert_eq!(first.len(), 1);
        client(|c| {
            c.sent = c.waiting.take();
        });
        offer(true, points.iter());
        assert!(client(|c| c.waiting.is_none()), "unchanged: nothing to send");
        offer(false, points.iter());
        assert_eq!(client(|c| c.waiting.clone()), Some(Vec::new()), "off: the empty list removes ours");
        client_reset();
    }
}
