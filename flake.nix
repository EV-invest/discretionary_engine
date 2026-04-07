{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    pre-commit-hooks.url = "github:cachix/git-hooks.nix";
    v_flakes.url = "github:valeratrades/v_flakes?ref=v1.5";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, pre-commit-hooks, v_flakes, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = builtins.trace "flake.nix sourced" [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
          extensions = [ "rust-src" "rust-analyzer" "rust-docs" "rustc-codegen-cranelift-preview" ];
        });
        pre-commit-check = pre-commit-hooks.lib.${system}.run (v_flakes.files.preCommit { inherit pkgs; });
        manifest = (pkgs.lib.importTOML ./discretionary_engine/Cargo.toml).package;
        pname = manifest.name;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;

        rs = v_flakes.rs {
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
            format = true;
            modules = {
              no_chrono = "false"; #dbg: used in code that's to be deprecated anyways
              prefer_ahash = "true";
            };
          };
        };
        github = v_flakes.github {
          inherit pkgs pname rs;
          enable = true;
          lastSupportedVersion = "nightly-2025-10-12";
          jobs.default = true;
          excalidraw."docs/arch.excalidraw".standalone = true;
          labels.extra = [
            # I think I should be grouping labels through color, right
            { name = "rm"; color = "0000ff"; description = "risk management side"; }
            { name = "integrations"; color = "20603D"; description = "all things related to how we access the underlying "; }
          ];
        };
        readme = v_flakes.readme-fw { inherit pkgs pname; defaults = true; lastSupportedVersion = "nightly-1.92"; rootDir = ./.; badges = [ "msrv" "crates_io" "docs_rs" "loc" "ci" ]; };

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
              cp -f ${(v_flakes.files.treefmt) {inherit pkgs;}} ./.treefmt.toml
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
