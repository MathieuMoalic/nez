{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = {
    flake-utils,
    rust-overlay,
    crane,
    advisory-db,
    nixpkgs,
    ...
  }: let
    perSystemOutputs = flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };

      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = ["rust-src" "rust-analyzer"];
      };

      # pkg-config helps find external dependencies
      nativeBuildInputs = with pkgs; [pkg-config rustls-libssl];

      nez = let
        craneLib = crane.mkLib {
          inherit rustToolchain pkgs;
          inherit (pkgs) callPackage;
        };

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          buildInputs = [rustToolchain] ++ nativeBuildInputs;
          cargoExtraArgs = "";
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        package =
          craneLib.buildPackage (commonArgs // {inherit cargoArtifacts;});

        checks = {
          clippy =
            craneLib.cargoClippy
            (commonArgs // {inherit cargoArtifacts;});
          test =
            craneLib.cargoNextest
            (commonArgs // {inherit cargoArtifacts;});
          fmt = craneLib.cargoFmt commonArgs;
          audit = craneLib.cargoAudit (commonArgs
            // {
              inherit advisory-db;
              cargoAuditExtraArgs = "--ignore RUSTSEC-2023-0071";
            });
        };
      in {inherit package checks;};

      app = flake-utils.lib.mkApp {drv = nez.package;};
    in {
      packages.default = nez.package;
      apps.default = app;

      devShells.default = pkgs.mkShell {
        inherit nativeBuildInputs;
        buildInputs = [rustToolchain pkgs.sqlx-cli pkgs.bacon];
        DATABASE_URL = "sqlite:./db.sqlite";
      };

      formatter = pkgs.writeShellApplication {
        name = "alejandra-nix-files";
        runtimeInputs = [pkgs.fd pkgs.alejandra];
        text = "fd --hidden --type f -e nix | xargs alejandra -q";
      };

      checks =
        {
          nix-files-are-formatted = pkgs.stdenvNoCC.mkDerivation {
            name = "fmt-check";
            dontBuild = true;
            src = ./.;
            doCheck = true;
            nativeBuildInputs = with pkgs; [fd alejandra];
            checkPhase = ''
              set -e
              fd --hidden --type f -e nix | xargs alejandra --check
            '';
            installPhase = ''mkdir "$out"'';
          };
        }
        // nez.checks;

      overlays.default = _final: _prev: {nez = app;};
    });
  in
    perSystemOutputs
    // {
      overlays.default = final: _prev: {
        nez = perSystemOutputs.packages.${final.stdenv.system}.default;
      };
    };
}
