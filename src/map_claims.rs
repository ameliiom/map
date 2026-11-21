// SPDX-License-Identifier: GPL-3.0-only
use crate::runtime::Dimension;

pub type Chunk = (Dimension, i32, i32);

fn origin(value: &str) -> Option<i32> {
    let n: i32 = value.parse().ok()?;
    (n.abs_diff(0) <= 30_000_000 && n % 16 == 0).then_some(n / 16)
}

pub fn own_chunks(line: &str) -> Option<Vec<Chunk>> {
    let (count, list) = line.strip_prefix("Your chunks (")?.split_once("): ")?;
    let count = count.split_once(" of ")?.0.parse::<usize>().ok()?;
    if count == 0 {
        return list.starts_with("none").then(Vec::new);
    }
    if count > 8192 {
        return None;
    }
    let chunks = list.split("; ").map(|entry| {
        let [dimension, "at", "x", x, "z", z]: [&str; 6] =
            entry.split_ascii_whitespace().collect::<Vec<_>>().try_into().ok()? else {
                return None;
            };
        let dimension = match dimension {
            "overworld" | "minecraft:overworld" => Dimension::Overworld,
            "the_nether" | "minecraft:the_nether" => Dimension::Nether,
            "the_end" | "minecraft:the_end" => Dimension::End,
            _ => return None,
        };
        Some((dimension, origin(x)?, origin(z)?))
    }).collect::<Option<Vec<_>>>()?;
    (chunks.len() == count).then_some(chunks)
}

pub fn here(line: &str) -> Option<([i32; 2], Option<&str>)> {
    let (range, owner) = line.strip_prefix("This chunk (x ")?.split_once(") is ")?;
    let (x, z) = range.split_once(", z ")?;
    let chunk = |range: &str| {
        let (from, to) = range.split_once(" to ")?;
        let chunk = origin(from)?;
        (to.parse::<i32>().ok()? == chunk * 16 + 15).then_some(chunk)
    };
    let owner = if owner == "free: /claim it." { None } else {
        let name = owner.strip_suffix("'s.")?;
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return None;
        }
        Some(name)
    };
    Some(([chunk(x)?, chunk(z)?], owner))
}

pub enum Reply {
    Own(Vec<Chunk>),
    Here(Chunk, Option<String>),
}

static REPLIES: std::sync::Mutex<Vec<Reply>> = std::sync::Mutex::new(Vec::new());

pub fn receive(line: &str, dimension: Dimension) {
    let reply = if let Some(chunks) = own_chunks(line) {
        Reply::Own(chunks)
    } else if let Some(([x, z], owner)) = here(line) {
        Reply::Here((dimension, x, z), owner.map(str::to_owned))
    } else {
        return;
    };
    let mut replies = REPLIES.lock().unwrap_or_else(|e| e.into_inner());
    if replies.len() < 8 {
        replies.push(reply);
    }
}

pub fn take_replies() -> Vec<Reply> {
    std::mem::take(&mut *REPLIES.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smp_claim_messages_are_complete_and_aligned() {
        assert_eq!(
            own_chunks("Your chunks (2 of 8): overworld at x -16 z 32; the_nether at x 0 z -48"),
            Some(vec![(Dimension::Overworld, -1, 2), (Dimension::Nether, 0, -3)]),
        );
        assert_eq!(own_chunks("Your chunks (0 of 4): none, /claim where you stand"), Some(vec![]));
        assert_eq!(own_chunks("Your chunks (2 of 8): overworld at x -16 z 32"), None);
        assert_eq!(here("This chunk (x -16 to -1, z 32 to 47) is Alex's."), Some(([-1, 2], Some("Alex"))));
        assert_eq!(here("This chunk (x -16 to 0, z 32 to 47) is Alex's."), None);
    }
}
