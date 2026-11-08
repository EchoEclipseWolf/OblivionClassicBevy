# Oblivion Classic Bevy

An in-progress recreation of *The Elder Scrolls IV: Oblivion* in Rust, using Bevy. The project aims to reproduce the original game's behavior while replacing its Windows-era platform and rendering interfaces with a modern host, starting on Linux and Windows.

## Current progress

The startup sequence through the initial player setup has been ported from the original Oblivion and checked against recorded behavior from the game. The project can display startup screens and menus, load game data, and render an early view of the Imperial Prison. Work is now moving into the New Game sequence, cell loading, and the systems needed to bring the world to life.

**It is not yet a playable game.** Gameplay, character creation, world simulation, and parts of rendering and audio are still being implemented or verified. Some tests compare behavior with captures from the original game; those comparisons do not yet cover everything.

The project uses original Oblivion game data, which is not included here. More detailed progress and remaining work are in [docs/open-items.md](docs/open-items.md).

## Libraries

Alongside Bevy, the project uses these libraries:

- **Avian 3D** (`avian3d`) — a physics library for Bevy, included as a dependency for physics work.
- **Rodio** — audio decoding and playback support.
- **flate2** — decompression for game data.
