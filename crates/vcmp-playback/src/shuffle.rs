//! The "less repeats" shuffle order.
//!
//! A cycle is a random permutation of the context, so every track plays once per
//! cycle. On top of that, the order keeps tracks by the same artist, then tracks from
//! the same album, out of adjacent slots where the context allows it:
//!
//! - The number of adjacent same-artist pairs is always the minimum possible,
//!   counting the pair formed with the track played before the cycle.
//! - Among the choices that keep that minimum, the one that also keeps same-album
//!   pairs to a minimum is preferred, step by step. That is exact when the artists
//!   leave the albums free (one artist, or all artists different) and a close
//!   heuristic otherwise.
//!
//! How: tracks are placed one at a time. A group (an artist or an album) that holds
//! more than about half of what is left decides where the conflicts must go, so each
//! step works out, in closed form, the fewest conflicts still unavoidable after each
//! candidate and only picks among the cheapest candidates. While no group is that
//! large, any track that differs from the previous one in both keys is optimal and
//! is drawn at random directly.

use std::collections::HashMap;
use std::sync::Arc;

use fastrand::Rng;

use crate::track::{QueueEntry, TrackRef};

/// Artist and album group of each context track, as small integers.
///
/// A track with an unknown artist (or album) gets a group of its own, shared only
/// with other entries of the very same track, so a duplicated playlist entry still
/// counts as a repeat.
#[derive(Clone, Debug, Default)]
pub(crate) struct Groups {
    artist: Vec<u32>,
    album: Vec<u32>,
    artists: usize,
    albums: usize,
}

#[derive(PartialEq, Eq, Hash)]
enum Key<'a> {
    Named(&'a str),
    Track(&'a TrackRef),
}

fn intern<'a>(keys: impl Iterator<Item = Key<'a>>) -> (Vec<u32>, usize) {
    let mut ids = HashMap::new();
    let groups = keys
        .map(|key| {
            let next = ids.len() as u32;
            *ids.entry(key).or_insert(next)
        })
        .collect();
    (groups, ids.len())
}

fn key<'a>(name: &'a Option<Arc<str>>, track: &'a TrackRef) -> Key<'a> {
    match name {
        Some(name) => Key::Named(name),
        None => Key::Track(track),
    }
}

impl Groups {
    pub(crate) fn new(entries: &[QueueEntry]) -> Groups {
        let (artist, artists) = intern(entries.iter().map(|e| key(&e.artist, &e.track)));
        let (album, albums) = intern(entries.iter().map(|e| key(&e.album, &e.track)));
        Groups {
            artist,
            album,
            artists,
            albums,
        }
    }
}

/// How many of the remaining tracks each group still holds.
struct Tally {
    count: Vec<usize>,
    /// `with_count[c]` is the number of groups holding exactly `c` tracks.
    with_count: Vec<usize>,
    max: usize,
}

impl Tally {
    fn new(groups: usize, members: impl Iterator<Item = u32>, total: usize) -> Tally {
        let mut count = vec![0; groups];
        for g in members {
            count[g as usize] += 1;
        }
        let mut with_count = vec![0; total + 1];
        for &c in &count {
            with_count[c] += 1;
        }
        let max = count.iter().copied().max().unwrap_or(0);
        Tally {
            count,
            with_count,
            max,
        }
    }

    fn take(&mut self, group: u32) {
        let c = self.count[group as usize];
        self.count[group as usize] = c - 1;
        self.with_count[c] -= 1;
        self.with_count[c - 1] += 1;
        if c == self.max && self.with_count[c] == 0 {
            self.max = c - 1;
        }
    }

    /// Whether some group is large enough (`2c + 1 >= remaining`) to constrain the
    /// next choice.
    fn constrains(&self, remaining: usize) -> bool {
        2 * self.max + 1 >= remaining
    }

    /// The constraining groups among `members`. There are at most three.
    fn constraining(&self, remaining: usize, members: impl Iterator<Item = u32>) -> Vec<u32> {
        let mut found = Vec::new();
        if self.constrains(remaining) {
            for g in members {
                if 2 * self.count[g as usize] + 1 >= remaining && !found.contains(&g) {
                    found.push(g);
                }
            }
        }
        found
    }

    /// Conflicts caused by placing a track of `group` next, plus the fewest conflicts
    /// unavoidable in the rest of the cycle afterwards.
    ///
    /// For `r` tracks left after a track of group `last`, the fewest adjacent pairs
    /// within one group `h` holding `c` of them is `2c - r` when `h == last` (it
    /// cannot go first) and `2c - r - 1` otherwise, floored at zero. Only a group
    /// holding about half of the tracks can make that positive, so only the
    /// `constraining` groups need checking.
    fn cost(&self, constraining: &[u32], remaining: usize, last: Option<u32>, group: u32) -> usize {
        let after = remaining - 1;
        let unavoidable = constraining
            .iter()
            .map(|&h| {
                let c = self.count[h as usize] - usize::from(h == group);
                if h == group {
                    (2 * c).saturating_sub(after)
                } else {
                    (2 * c).saturating_sub(after + 1)
                }
            })
            .max()
            .unwrap_or(0);
        usize::from(last == Some(group)) + unavoidable
    }
}

/// Random tries for an unconstrained pick before falling back to a full scan.
const FAST_TRIES: usize = 32;

/// Returns `items` (indices into the context) in "less repeats" order. `previous`
/// is the context index played just before this cycle starts, if any.
pub(crate) fn spread(
    items: &[usize],
    groups: &Groups,
    previous: Option<usize>,
    rng: &mut Rng,
) -> Vec<usize> {
    let n = items.len();
    let mut artists = Tally::new(groups.artists, items.iter().map(|&i| groups.artist[i]), n);
    let mut albums = Tally::new(groups.albums, items.iter().map(|&i| groups.album[i]), n);
    let mut last_artist = previous.map(|p| groups.artist[p]);
    let mut last_album = previous.map(|p| groups.album[p]);
    let mut remaining = items.to_vec();
    let mut order = Vec::with_capacity(n);
    let mut cheapest = Vec::new();

    while !remaining.is_empty() {
        let r = remaining.len();
        let at = 'pick: {
            if !artists.constrains(r) && !albums.constrains(r) {
                for _ in 0..FAST_TRIES {
                    let at = rng.usize(..r);
                    let item = remaining[at];
                    if Some(groups.artist[item]) != last_artist
                        && Some(groups.album[item]) != last_album
                    {
                        break 'pick at;
                    }
                }
            }
            let artist_groups =
                artists.constraining(r, remaining.iter().map(|&i| groups.artist[i]));
            let album_groups = albums.constraining(r, remaining.iter().map(|&i| groups.album[i]));
            let mut best = (usize::MAX, usize::MAX);
            cheapest.clear();
            for (at, &item) in remaining.iter().enumerate() {
                let cost = (
                    artists.cost(&artist_groups, r, last_artist, groups.artist[item]),
                    albums.cost(&album_groups, r, last_album, groups.album[item]),
                );
                if cost < best {
                    best = cost;
                    cheapest.clear();
                }
                if cost == best {
                    cheapest.push(at);
                }
            }
            cheapest[rng.usize(..cheapest.len())]
        };
        let item = remaining.swap_remove(at);
        let (artist, album) = (groups.artist[item], groups.album[item]);
        artists.take(artist);
        albums.take(album);
        last_artist = Some(artist);
        last_album = Some(album);
        order.push(item);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Context sizes checked against every possible order.
    const EXHAUSTIVE: usize = 6;

    fn entries(artists: &[u8], albums: &[u8]) -> Vec<QueueEntry> {
        artists
            .iter()
            .zip(albums)
            .enumerate()
            .map(|(i, (a, b))| {
                QueueEntry::new(TrackRef::local(format!("/t/{i}")))
                    .with_artist(format!("artist {a}"))
                    .with_album(format!("album {b}"))
            })
            .collect()
    }

    fn pairs(order: &[usize], previous: Option<usize>, key: &[u8]) -> usize {
        let first = match (previous, order.first()) {
            (Some(p), Some(&f)) => usize::from(key[p] == key[f]),
            _ => 0,
        };
        first + order.windows(2).filter(|w| key[w[0]] == key[w[1]]).count()
    }

    fn permutations(items: &[usize]) -> Vec<Vec<usize>> {
        if items.len() <= 1 {
            return vec![items.to_vec()];
        }
        let mut all = Vec::new();
        for i in 0..items.len() {
            let mut rest = items.to_vec();
            let first = rest.remove(i);
            for mut tail in permutations(&rest) {
                tail.insert(0, first);
                all.push(tail);
            }
        }
        all
    }

    /// The fewest (artist pairs, album pairs) over all orders, lexicographically.
    fn optimum(
        all: &[Vec<usize>],
        previous: Option<usize>,
        artists: &[u8],
        albums: &[u8],
    ) -> (usize, usize) {
        all.iter()
            .map(|p| (pairs(p, previous, artists), pairs(p, previous, albums)))
            .min()
            .unwrap()
    }

    /// Every way to label `n` tracks with groups, up to renaming the groups.
    fn labelings(n: usize) -> Vec<Vec<u8>> {
        let mut out = vec![vec![]];
        for _ in 0..n {
            out = out
                .into_iter()
                .flat_map(|labels: Vec<u8>| {
                    let next = labels.iter().copied().max().map_or(0, |m| m + 1);
                    (0..=next).map(move |l| {
                        let mut labels = labels.clone();
                        labels.push(l);
                        labels
                    })
                })
                .collect();
        }
        out
    }

    fn assert_permutation(order: &[usize], items: &[usize]) {
        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        let mut expected = items.to_vec();
        expected.sort_unstable();
        assert_eq!(sorted, expected, "not a permutation: {order:?}");
    }

    #[test]
    fn artist_pairs_are_always_minimal() {
        let mut rng = Rng::with_seed(7);
        for n in 1..=EXHAUSTIVE {
            let items: Vec<usize> = (0..n).collect();
            let all = permutations(&items);
            for artists in labelings(n) {
                let albums: Vec<u8> = (0..n as u8).collect();
                let groups = Groups::new(&entries(&artists, &albums));
                // Also cover a cycle that follows a track: any track of the context.
                for previous in std::iter::once(None).chain((0..n).map(Some)) {
                    let best = optimum(&all, previous, &artists, &albums).0;
                    for _ in 0..4 {
                        let order = spread(&items, &groups, previous, &mut rng);
                        assert_permutation(&order, &items);
                        assert_eq!(
                            pairs(&order, previous, &artists),
                            best,
                            "artists {artists:?}, previous {previous:?}, order {order:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn album_pairs_are_minimal_for_a_single_artist() {
        let mut rng = Rng::with_seed(11);
        for n in 1..=EXHAUSTIVE {
            let items: Vec<usize> = (0..n).collect();
            let all = permutations(&items);
            for albums in labelings(n) {
                let artists = vec![0; n];
                let groups = Groups::new(&entries(&artists, &albums));
                for previous in std::iter::once(None).chain((0..n).map(Some)) {
                    let best = optimum(&all, previous, &artists, &albums);
                    for _ in 0..4 {
                        let order = spread(&items, &groups, previous, &mut rng);
                        assert_permutation(&order, &items);
                        let got = (
                            pairs(&order, previous, &artists),
                            pairs(&order, previous, &albums),
                        );
                        assert_eq!(got, best, "albums {albums:?}, order {order:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn album_pairs_are_minimal_when_every_artist_differs() {
        // A compilation: one album, many artists, plus other albums mixed in.
        let mut rng = Rng::with_seed(13);
        for n in 1..=EXHAUSTIVE {
            let items: Vec<usize> = (0..n).collect();
            let all = permutations(&items);
            for albums in labelings(n) {
                let artists: Vec<u8> = (0..n as u8).collect();
                let groups = Groups::new(&entries(&artists, &albums));
                let best = optimum(&all, None, &artists, &albums);
                for _ in 0..4 {
                    let order = spread(&items, &groups, None, &mut rng);
                    let got = (pairs(&order, None, &artists), pairs(&order, None, &albums));
                    assert_eq!(got, best, "albums {albums:?}, order {order:?}");
                }
            }
        }
    }

    #[test]
    fn mixed_albums_never_exceed_the_artist_minimum() {
        // With albums nested in artists, an album pair is also an artist pair, so the
        // artist guarantee bounds them; check the combined behaviour on random input.
        let mut rng = Rng::with_seed(17);
        let all: Vec<Vec<Vec<usize>>> = (0..=EXHAUSTIVE)
            .map(|n| permutations(&(0..n).collect::<Vec<_>>()))
            .collect();
        for _ in 0..2000 {
            let n = rng.usize(1..=EXHAUSTIVE);
            let artists: Vec<u8> = (0..n).map(|_| rng.u8(0..3)).collect();
            let albums: Vec<u8> = artists.iter().map(|&a| a * 2 + rng.u8(0..2)).collect();
            let groups = Groups::new(&entries(&artists, &albums));
            let items: Vec<usize> = (0..n).collect();
            let best = optimum(&all[n], None, &artists, &albums);
            let order = spread(&items, &groups, None, &mut rng);
            assert_eq!(pairs(&order, None, &artists), best.0);
            assert!(pairs(&order, None, &albums) <= pairs(&order, None, &artists));
        }
    }

    #[test]
    fn a_subset_is_spread_after_the_previous_track() {
        let artists = [0, 0, 1, 1, 2, 2];
        let albums = [0, 0, 1, 1, 2, 2];
        let groups = Groups::new(&entries(&artists, &albums));
        let mut rng = Rng::with_seed(3);
        for _ in 0..100 {
            let order = spread(&[1, 2, 3, 4, 5], &groups, Some(0), &mut rng);
            assert_permutation(&order, &[1, 2, 3, 4, 5]);
            assert_eq!(pairs(&order, Some(0), &artists), 0, "{order:?}");
        }
    }

    #[test]
    fn unknown_artists_only_match_the_same_track() {
        let track = TrackRef::local("/t/same");
        let context = vec![
            QueueEntry::new(track.clone()),
            QueueEntry::new(TrackRef::local("/t/other")),
            QueueEntry::new(track),
        ];
        let groups = Groups::new(&context);
        let mut rng = Rng::with_seed(5);
        for _ in 0..50 {
            // The duplicated track must not play twice in a row.
            assert_eq!(spread(&[0, 1, 2], &groups, None, &mut rng)[1], 1);
        }
    }

    #[test]
    fn large_contexts_stay_spread_and_fast() {
        let n = 20_000;
        let context: Vec<QueueEntry> = (0..n)
            .map(|i| {
                QueueEntry::new(TrackRef::local(format!("/t/{i}")))
                    .with_artist(format!("a{}", i % 1000))
                    .with_album(format!("b{}", i % 2000))
            })
            .collect();
        let groups = Groups::new(&context);
        let items: Vec<usize> = (0..n).collect();
        let start = std::time::Instant::now();
        let order = spread(&items, &groups, None, &mut Rng::with_seed(1));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        assert_permutation(&order, &items);
        let key: Vec<usize> = (0..n).map(|i| i % 1000).collect();
        assert!(order.windows(2).all(|w| key[w[0]] != key[w[1]]));
    }
}
