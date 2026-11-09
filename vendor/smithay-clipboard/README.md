> **Copie modifiée pour SterMusic** : `src/dnd.rs` et le `DataDeviceHandler` de `src/state.rs`
> acceptent les fichiers glissés sur la fenêtre (KWin n'envoie le glisser qu'au premier
> `wl_data_device` de l'application, qui est celui du presse-papiers). Branchée par
> `[patch.crates-io]` dans le `Cargo.toml` de SterMusic.

[![crates.io](https://img.shields.io/crates/v/smithay-clipboard.svg)](https://crates.io/crates/smithay-clipboard)


# Smithay Clipboard

This crate provides access to the Wayland clipboard for applications
already using some sort of GUI toolkit or a windowing library, like
[winit](https://github.com/rust-windowing/winit), since you should
have some surface around to receive keyboard/pointer events.

If you want to access clipboard from the CLI or to write clipboard manager,
this is not what you're looking for.

## Documentation

The documentation for the master branch is [available online](https://smithay.github.io/smithay-clipboard/).

The documentation for the releases can be found on [docs.rs](https://docs.rs/smithay-clipboard).

## Contact Us

If you have questions or want to discuss the project with us, join our chatroom on matrix:
[#sctk:matrix.org](https://matrix.to/#/#sctk:matrix.org).
