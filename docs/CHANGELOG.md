# Changelog

All notable changes to NovaSDR are documented in this file.

## v0.1.10

- Added configurable default SSB passband edges via `receivers[].input.defaults.ssb_lowcut_hz` and `receivers[].input.defaults.ssb_highcut_hz`.
- Updated the setup/configure wizard to prompt for SSB low/high cut when the default modulation is USB/LSB.
- Updated configuration documentation for the new SSB defaults.
- Frontend: apply server-provided SSB defaults on USB/LSB mode changes; improved background image handling (`background.jpg` / `background.png`).


