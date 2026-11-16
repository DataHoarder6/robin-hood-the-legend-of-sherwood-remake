# Libraries the `robin` binary resolves at runtime.
#
# winit dlopens Xlib/wayland and wgpu dlopens the GL/Vulkan loaders, so a
# package cannot rely on link-time RPATH alone; the devShell exports these on
# LD_LIBRARY_PATH and the package wrapper does the same. Keep one list: a
# missing entry is a startup crash, not a link error.
pkgs:
with pkgs;
[
  libx11
  libxcursor
  libxi
  libxrandr
  libxcb
  libxkbcommon
  wayland
  alsa-lib
  dbus
  libpulseaudio
  systemd
  mesa
  vulkan-loader
]
