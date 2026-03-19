{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    pre-commit-hooks.url = "github:cachix/git-hooks.nix";
    v-flakes.url = "github:valeratrades/v_flakes?ref=v1.4";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, pre-commit-hooks, v-flakes, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = builtins.trace "flake.nix sourced" [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
          extensions = [ "rust-src" "rust-analyzer" "rust-docs" "rustc-codegen-cranelift-preview" ];
        });
        pre-commit-check = pre-commit-hooks.lib.${system}.run (v-flakes.files.preCommit { inherit pkgs; });
        manifest = (pkgs.lib.importTOML ./discretionary_engine/Cargo.toml).package;
        pname = manifest.name;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;

        rs = v-flakes.rs {
          inherit pkgs rust;
          cranelift = false; # cranelift disabled due to aws-lc-rs incompatibility
          tracey = false; # feels raw. And kinda pointless, as I don't see shit enforced. Might not understand it well enough, but starting to think it superflous
          build = {
            enable = true;
            workspace = {
              "./discretionary_engine" = [ "git_version" "log_directives" ];
              "./discretionary_engine_strategy" = [ "git_version" "log_directives" ];
            };
          };
          style = {
            modules = {
              no_chrono = "false"; #dbg: used in code that's to be deprecated anyways
            };
          };
        };
        github = v-flakes.github {
          inherit pkgs pname rs;
          enable = true;
          lastSupportedVersion = "nightly-2025-10-12";
          jobs.default = true;
          langs = [ "rs" ];
          excalidraw."docs/arch.excalidraw".standalone = true;
          labels.extra = [
            # I think I should be grouping labels through color, right
            { name = "rm"; color = "0000ff"; description = "risk management side"; }
            { name = "integrations"; color = "20603D"; description = "all things related to how we access the underlying "; }
          ];
        };
        readme = v-flakes.readme-fw { inherit pkgs pname; defaults = true; lastSupportedVersion = "nightly-1.92"; rootDir = ./.; badges = [ "msrv" "crates_io" "docs_rs" "loc" "ci" ]; };

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
              cp -f ${(v-flakes.files.treefmt) {inherit pkgs;}} ./.treefmt.toml
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
