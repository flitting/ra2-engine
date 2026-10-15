//! The drive locomotor's tracks: the paths a vehicle's centre follows, point by
//! point, as it drives straight, round a bend, or swings round within its cell.
//!
//! The tables themselves are the engine's data and are read out of the user's
//! the original game when the game starts; they are not part of this repository.

/// One point on a track: an offset in leptons from the centre of the cell the track
/// ends in, and the facing the unit has there.
pub type TrackPoint = (i32, i32, u8);

pub struct Track {
    pub points: Vec<TrackPoint>,
    /// The point at which a following curve may be joined, or -1.
    pub jump: i32,
    /// The point, counted from one, at which this track is joined from another, or 0.
    pub entry: i32,
    /// The point at which the unit passes into the next cell, or -1.
    pub cell: i32,
}

/// Which tracks to use for one pair of directions.
#[derive(Clone, Copy, Debug)]
pub struct Selector {
    /// The track for driving, or 0 if the turn is too sharp for one.
    pub track: u8,
    /// The short track for making the turn within one cell, or 0.
    pub start: u8,
    /// The facing the unit ends with.
    pub facing: u8,
    flags: u8,
}

const TRANSPOSE: u8 = 1;
const FLIP_X: u8 = 2;
const FLIP_Y: u8 = 4;
const TWO_CELLS: u8 = 8;

impl Selector {
    /// Whether the main track runs through one cell and ends in the next.
    pub fn two_cells(&self) -> bool {
        self.flags & TWO_CELLS != 0
    }

    /// A track point turned to suit this pair of directions. Tracks are stored for
    /// one orientation and mirrored for the others.
    // The mirroring rules are those of the drive code in Westwood's released Red
    // Alert 1 source; applied to the original game's tables they give the right start point
    // and facing for all eight straight directions (see the tests).
    pub fn place(&self, (mut x, mut y, mut facing): TrackPoint) -> TrackPoint {
        if self.flags & TRANSPOSE != 0 {
            std::mem::swap(&mut x, &mut y);
            facing = 192u8.wrapping_sub(facing);
        }
        if self.flags & FLIP_X != 0 {
            x = -x;
            facing = facing.wrapping_neg();
        }
        if self.flags & FLIP_Y != 0 {
            y = -y;
            facing = 128u8.wrapping_sub(facing);
        }
        (x, y, facing)
    }
}

pub struct TrackTables {
    /// Tracks by number; number 0 is unused.
    pub tracks: Vec<Track>,
    /// Indexed by first direction times eight plus second direction.
    pub selector: Vec<Selector>,
}

// DATA: where the tables sit in the original game (Steam build). The executable's read-only
// data is mapped at its file offset plus this base.
const IMAGE_BASE: usize = 0x40_0000;
const TRACKS_AT: usize = 0x7a_00a0;
const TRACK_COUNT: usize = 14;
const SELECTOR_AT: usize = 0x7a_0180;
// The first 64 are for pairs of directions; the three after are special, the last
// of them (66) the way out of a war factory.
const SELECTOR_COUNT: usize = 67;
/// The selector entry for driving out of a war factory.
pub const FACTORY_EXIT: usize = 66;

fn i32_at(data: &[u8], address: usize) -> Option<i32> {
    let at = address.checked_sub(IMAGE_BASE)?;
    Some(i32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

impl TrackTables {
    /// Reads the tables from the bytes of the original game. Returns `None` if they are not
    /// where this build keeps them.
    pub fn from_exe(exe: &[u8]) -> Option<Self> {
        let headers: Vec<(usize, i32, i32, i32)> = (0..TRACK_COUNT)
            .map(|index| {
                let at = TRACKS_AT + index * 16;
                Some((
                    i32_at(exe, at)? as u32 as usize,
                    i32_at(exe, at + 4)?,
                    i32_at(exe, at + 8)?,
                    i32_at(exe, at + 12)?,
                ))
            })
            .collect::<Option<_>>()?;

        // A track's points run up to the start of whichever table comes next.
        let mut starts: Vec<usize> = headers.iter().map(|h| h.0).filter(|&p| p != 0).collect();
        starts.push(TRACKS_AT);
        starts.sort_unstable();
        let mut tracks = Vec::with_capacity(TRACK_COUNT);
        for &(pointer, jump, entry, cell) in &headers {
            let mut points = Vec::new();
            if pointer != 0 {
                let end = *starts.iter().find(|&&start| start > pointer)?;
                for at in (pointer..end).step_by(12) {
                    let facing = i32_at(exe, at + 8)?;
                    points.push((i32_at(exe, at)?, i32_at(exe, at + 4)?, facing as u8));
                }
                // The table is padded with empty entries after the last point.
                while points.last() == Some(&(0, 0, 0)) {
                    points.pop();
                }
            }
            tracks.push(Track {
                points,
                jump,
                entry,
                cell,
            });
        }

        let selector: Vec<Selector> = (0..SELECTOR_COUNT)
            .map(|index| {
                let at = SELECTOR_AT + index * 12;
                let first = i32_at(exe, at)?;
                Some(Selector {
                    track: first as u8,
                    start: (first >> 8) as u8,
                    facing: i32_at(exe, at + 4)? as u8,
                    flags: i32_at(exe, at + 8)? as u8,
                })
            })
            .collect::<Option<_>>()?;

        // Sanity checks, so that a different build is refused and not misread.
        let straight = tracks.get(1)?;
        let valid = straight.points.first() == Some(&(0, 245, 0))
            && selector[0].track == 1
            && selector
                .iter()
                .all(|entry| (entry.track as usize) < TRACK_COUNT && (entry.start as usize) < TRACK_COUNT);
        valid.then_some(Self { tracks, selector })
    }
}
