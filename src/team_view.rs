//! What Map draws of Team, when Team is installed: its teammates and live pings on this client,
//! read through Team's own API (`team::with_mates`, `team::with_markers`, `team::colour`) each
//! time they are drawn. Team owns the data and the colours; these are only the drawing's view.
//! Without Team (the launcher's `team` feature off) both lists are empty.
#![allow(unexpected_cfgs)]

/// A teammate: who, where, and their colour (ARGB).
pub struct Mate {
    pub name: String,
    pub at: [f64; 3],
    pub dimension: String,
    pub colour: u32,
}

/// What a ping marks, which picks its icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Place,
    Danger,
    Entity,
    Loot,
}

/// A live ping: who placed it (their colour), where, and what it marks.
pub struct Ping {
    pub owner: String,
    pub at: [f64; 3],
    pub dimension: String,
    pub colour: u32,
    pub mark: Mark,
    /// The block pinged, for a loot ping's icon (`minecraft:diamond_ore`).
    pub block: Option<String>,
}

/// The teammates, nearest data first as Team keeps them.
pub fn mates() -> Vec<Mate> {
    #[cfg(feature = "team")]
    {
        return team::with_mates(|mates| {
            mates
                .iter()
                .map(|mate| Mate {
                    name: mate.member.name.clone(),
                    at: [mate.member.x, mate.member.y, mate.member.z],
                    dimension: mate.member.dimension.clone(),
                    colour: team::colour(&mate.member.name),
                })
                .collect()
        });
    }
    #[allow(unreachable_code)]
    Vec::new()
}

/// The pings Team shows now.
pub fn pings() -> Vec<Ping> {
    #[cfg(feature = "team")]
    {
        use team::payloads::PingKind;
        return team::with_markers(|markers| {
            markers
                .iter()
                .map(|shown| {
                    let marker = &shown.marker;
                    Ping {
                        owner: marker.owner.clone(),
                        at: [marker.x, marker.y, marker.z],
                        dimension: marker.dimension.clone(),
                        colour: team::colour(&marker.owner),
                        mark: match marker.kind {
                            PingKind::Place => Mark::Place,
                            PingKind::Danger => Mark::Danger,
                            PingKind::Entity => Mark::Entity,
                            PingKind::Loot => Mark::Loot,
                        },
                        block: marker.block.clone(),
                    }
                })
                .collect()
        });
    }
    #[allow(unreachable_code)]
    Vec::new()
}
