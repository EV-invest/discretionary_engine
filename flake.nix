{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    pre-commit-hooks.url = "github:cachix/git-hooks.nix";
    v-utils.url = "github:valeratrades/.github";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, pre-commit-hooks, v-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = builtins.trace "flake.nix sourced" [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
          extensions = [ "rust-src" "rust-analyzer" "rust-docs" "rustc-codegen-cranelift-preview" ];
        });
        pre-commit-check = pre-commit-hooks.lib.${system}.run (v-utils.files.preCommit { inherit pkgs; });
        manifest = (pkgs.lib.importTOML ./discretionary_engine/Cargo.toml).package;
        pname = manifest.name;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;

        rs = v-utils.rs {
          inherit pkgs;
          cranelift = false; # cranelift disabled due to aws-lc-rs incompatibility
          tracey = true;
          build = {
            enable = true;
            workspace = {
              "./discretionary_engine" = [ "git_version" "log_directives" ];
            };
          };
        };
        github = v-utils.github {
          inherit pkgs pname;
          inherit (rs) traceyCheck;
          lastSupportedVersion = "nightly-2025-10-12";
          jobs.default = true;
          langs = [ "rs" ];
          labels.extra = [{ name = "rm"; color = "0000ff"; }];
        };
        readme = v-utils.readme-fw { inherit pkgs pname; defaults = true; lastSupportedVersion = "nightly-1.92"; rootDir = ./.; badges = [ "msrv" "crates_io" "docs_rs" "loc" "ci" ]; };
      in
      {
        packages =
          let
            rustc = rust;
            cargo = rust;
            rustPlatform = pkgs.makeRustPlatform {
              inherit rustc cargo stdenv;
            };
          in
          {
            default = rustPlatform.buildRustPackage {
              inherit pname;
              version = manifest.version;

              buildInputs = with pkgs; [
                openssl.dev
              ];
              nativeBuildInputs = with pkgs; [ pkg-config ];

              cargoLock.lockFile = ./Cargo.lock;
              src = self;
            };
          };

        devShells.default = with pkgs; mkShell {
          inherit stdenv;
          shellHook =
            pre-commit-check.shellHook +
            github.shellHook +
            rs.shellHook +
            readme.shellHook +
            ''
              cp -f ${(v-utils.files.treefmt) {inherit pkgs;}} ./.treefmt.toml
            '';

          env = {
            RUST_BACKTRACE = 1;
            RUST_LIB_BACKTRACE = 0;
          };

          packages = [
            mold
            openssl
            pkg-config
            rust
          ] ++ pre-commit-check.enabledPackages ++ github.enabledPackages ++ rs.enabledPackages;
        };
      }
    );
}
