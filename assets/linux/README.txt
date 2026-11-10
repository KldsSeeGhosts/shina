Shina for Linux x86_64

Extract the archive, then run ./Shina/shina.
This build targets Ubuntu 24.04 or newer, or another distribution with glibc 2.39+.
It needs a Vulkan-capable graphics driver, a Wayland or X11 desktop, and the
existing Pi/CPA enrollment used by the other Shina builds.

This is a portable development build. --install and the in-app updater currently
support Windows and macOS only. To update Linux, download and extract the newer
archive. Conversation data stays outside this folder under $XDG_DATA_HOME/Shina
or ~/.local/share/Shina.
