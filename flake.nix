# Copyright (c) 2026 Alwin Berger
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was fully AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: Xiaomi mimo-v2.6-pro
{
  description = "Doctor Whodunit (Weeping-Angel) development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      # `nix fmt`
      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };

          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-analyzer"
              "rust-src"
            ];
          };
          protoc = pkgs.protobuf;
        in
        {
          default = pkgs.mkShell {
            name = "doctor-whodunit";

            packages = [
              rustToolchain
              protoc
              pkgs.git
              pkgs.clang
              # Python with matplotlib for product/scripts/plot-asc
              (pkgs.python3.withPackages (ps: [ ps.matplotlib ]))
              # zenoh may call pkg-config at build time for OpenSSL linkage
              pkgs.pkg-config
              pkgs.openssl
              pkgs.toxiproxy
            ];

            env = {
              # prost-build (via tonic-build) shells out to protoc through
              # $PROTOC; it never auto-locates the well-known types dir.
              PROTOC = "${protoc}/bin/protoc";
              # demo/services/build.rs adds this dir to the -I paths when set,
              # so google/protobuf/*.proto (e.g. timestamp.proto) resolve on
              # Nix, where /usr/include does not exist.
              PROTOC_INCLUDE = "${protoc}/include";
              # iceoryx2's platform layer (fault-lib path deps) generates its
              # Linux bindings with bindgen, which needs libclang and glibc
              # headers — the plain libclang.so does not see glibc.dev.
              LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
              BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.glibc.dev}/include";
              RUST_BACKTRACE = "1";
            };

            shellHook = ''
              echo "Doctor Whodunit devshell | $(rustc --version) | $(protoc --version)"
            '';
          };
        }
      );
    };
}
