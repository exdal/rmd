{
  pkgs ? import <nixpkgs> { overlays = [ (import <rust-overlay>) ]; }
}:
let
  toolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override (previous: {
    extensions = previous.extensions ++ [ "rust-src" "rust-analyzer" ];
  });
in
pkgs.mkShell {
  nativeBuildInputs = [
    toolchain
    pkgs.pkg-config
    pkgs.libX11
    pkgs.libXcursor
    pkgs.libXrandr
    pkgs.libXi
    pkgs.libxcb
    pkgs.libxkbcommon
    pkgs.vulkan-loader
    pkgs.wayland
    pkgs.shader-slang
    pkgs.llvmPackages.libclang
    pkgs.zenity
  ];

  buildInputs = [
    pkgs.python315
  ];

  SLANG_INCLUDE_DIR = "${pkgs.shader-slang.dev}/include";
  SLANG_LIB_DIR = "${pkgs.shader-slang}/lib";
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.glibc.dev}/include";

  shellHook = ''
    export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${pkgs.wayland}/lib";
    export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${pkgs.libxkbcommon}/lib";
    export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${pkgs.vulkan-loader}/lib";
    export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:${pkgs.shader-slang}/lib";
  '';
}
