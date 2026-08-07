{
  inputs = {
    v_flakes.url = "github:valeratrades/v_flakes?ref=v1.6";
  };

  outputs = { self, v_flakes, ... }:
    let
      inherit (v_flakes) flake-utils pre-commit-hooks;
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import v_flakes.default_nixpkgs { inherit system; };
        rust = v_flakes.rs.default_nightly system;
        pre-commit-check = pre-commit-hooks.lib.${system}.run (v_flakes.files.preCommit { inherit pkgs; });
        manifest = (pkgs.lib.importTOML ./discretionary_engine/Cargo.toml).package;
        pname = manifest.name;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;

        rs = v_flakes.rs {
          inherit pkgs rust;
          cranelift = false; # cranelift disabled due to aws-lc-rs incompatibility
          tracey = true; # feels a bit raw though, - enforcement is a bit weak
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
          excludeDirs = [ "libs/nautilus_trader" ];
          lastSupportedVersion = "nightly-2025-10-12";
          jobs.default = true;
          labels.extra = [
            # I think I should be grouping labels through color, right
            { name = "rm"; color = "0000ff"; description = "risk management side"; }
            { name = "integrations"; color = "20603D"; description = "all things related to how we access the underlying "; }
          ];
        };
        readme = v_flakes.readme-fw { inherit pkgs pname; defaults = true; lastSupportedVersion = "nightly-1.92"; rootDir = ./.; badges = [ "msrv" "crates_io" "docs_rs" "loc" "ci" ]; };
        combined = v_flakes.utils.combine { inherit rust; modules = [ rs github readme ]; };

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
            combined.shellHook +
            ''
              cp -f ${(v_flakes.files.treefmt) {inherit pkgs; extend = { global.excludes.augment = [ "libs/**" ]; };}} ./.treefmt.toml
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
          ] ++ pre-commit-check.enabledPackages ++ combined.enabledPackages;
        };
      }
    );
}
