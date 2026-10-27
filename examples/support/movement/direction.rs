//! Direction names shared by browser and WASI keyboard transports.
use crate::movement_example::model::Direction;
pub fn parse(value: &str) -> Result<Direction, String> {
    match value {
        "north" | "w" => Ok(Direction::North),
        "south" | "s" => Ok(Direction::South),
        "east" | "d" => Ok(Direction::East),
        "west" | "a" => Ok(Direction::West),
        "stop" | " " => Ok(Direction::Stop),
        _ => Err("Direction must be north, south, east, west or stop".into()),
    }
}
