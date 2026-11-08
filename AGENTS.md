# mactop

This is a personal Apple Silicon monitor with terminal and native menu-bar
interfaces, not a general-purpose monitoring framework. CPU, memory, swap, and
processes are the deliberate scope; GPU/power monitoring and privileged or
subprocess-based collection are not planned features.

The [development notes](docs/DEVELOPMENT.md) record the layout and measurement
contracts, hardware validation limits, and run/check commands. Keep those details
there rather than duplicating them in agent instructions.
