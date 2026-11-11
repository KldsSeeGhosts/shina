Shina for Linux x86_64

Extract the archive, then run ./Shina/shina.
This build targets Ubuntu 24.04 or newer, or another distribution with glibc 2.39+.
It needs a Vulkan-capable graphics driver, a Wayland or X11 desktop, and the
existing Pi/CPA enrollment used by the other Shina builds.

This is a portable development build. --install and the in-app updater currently
support Windows and macOS only. Linux hosts can run ./Shina/shina update --yes
to download, verify and install a newer release with their existing gh login.
The update refuses active work and coordinates a server or service restart.
Use ./Shina/shina service install to opt into a systemd user service.
Lingering changes only with service install --enable-linger, without sudo.
Conversation data stays outside this folder under $XDG_DATA_HOME/Shina
or ~/.local/share/Shina.
