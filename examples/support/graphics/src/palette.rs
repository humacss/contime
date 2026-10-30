//! One identity color per client, shared by every part of the presentation.
use bevy::prelude::*;

const CLIENTS: [[u8; 3]; 5] = [
    [242, 199, 107], // Gold: keyboard-controlled player.
    [98, 214, 164],  // Mint.
    [101, 185, 241], // Sky.
    [239, 141, 131], // Coral.
    [187, 157, 233], // Lilac.
];
fn rgb(client: usize) -> [f32; 3] {
    CLIENTS[client.saturating_sub(1) % CLIENTS.len()].map(|c| f32::from(c) / 255.0)
}
pub fn client(client: usize, alpha: f32) -> Color {
    let [r, g, b] = rgb(client);
    Color::srgba(r, g, b, alpha)
}
/// Lighter input markers keep the same hue as their client's path.
pub fn input(client: usize, alpha: f32) -> Color {
    let [r, g, b] = rgb(client).map(|c| c + (1.0 - c) * 0.32);
    Color::srgba(r, g, b, alpha)
}
/// Opaque paths cover identical revised geometry, even as older paths dim.
pub fn path(client: usize, age: f32) -> Color {
    let [r, g, b] = rgb(client).map(|c| c * (0.65 * age.clamp(0.0, 1.0)));
    Color::srgb(r, g, b)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_supported_client_has_a_distinct_consistent_palette() {
        assert_eq!(CLIENTS.len(), crate::report::MAX_CLIENTS);
        for id in 1..=CLIENTS.len() {
            let base = client(id, 1.0).to_srgba();
            let icon = input(id, 0.6).to_srgba();
            let trail = path(id, 1.0).to_srgba();
            assert_eq!(icon.alpha, 0.6);
            assert_eq!(trail.alpha, 1.0);
            for (b, i, p) in [(base.red, icon.red, trail.red), (base.green, icon.green, trail.green), (base.blue, icon.blue, trail.blue)] {
                assert!(i > b && b > p);
            }
            for other in 1..id {
                assert_ne!(client(id, 1.0), client(other, 1.0));
            }
            assert_ne!(path(id, 0.2), path(id, 1.0));
        }
    }
}
