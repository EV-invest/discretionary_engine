{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    pre-commit-hooks.url = "github:cachix/git-hooks.nix";
    v-utils.url = "github:valeratrades/.github?ref=v1.4";
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
        github = v-utils.github {
          inherit pkgs pname rs;
          lastSupportedVersion = "nightly-2025-10-12";
          jobs.default = true;
          langs = [ "rs" ];
          gitignore.extra = "_scripts/node_modules";
          labels.extra = [
            # I think I should be grouping labels through color, right
            { name = "rm"; color = "0000ff"; description = "risk management side"; }
            { name = "integrations"; color = "20603D"; description = "all things related to how we access the underlying "; }
          ];
        };
        readme = v-utils.readme-fw { inherit pkgs pname; defaults = true; lastSupportedVersion = "nightly-1.92"; rootDir = ./.; badges = [ "msrv" "crates_io" "docs_rs" "loc" "ci" ]; };

        # ---------------------------------------------------------------------------
        # Excalidraw tooling — all scripts embedded so `nix develop` is self-contained.
        # Copied into _scripts/ (gitignored) on shell entry; npm run via flake-provided nodejs.
        # Commands: `ex` (open editor), `ex-to-md` (→ docs/arch.md), `md-to-ex` (→ docs/arch.excalidraw)
        # ---------------------------------------------------------------------------

        excalidrawPackageJson = pkgs.writeText "ex-package.json" ''
          {
            "name": "de-excalidraw-tools",
            "version": "1.0.0",
            "type": "module",
            "private": true,
            "dependencies": {
              "@excalidraw/mermaid-to-excalidraw": "*",
              "jsdom": "*"
            }
          }
        '';

        excalidrawAppHtml = pkgs.writeText "excalidraw-app.html" ''
          <!DOCTYPE html>
          <html>
          <head>
            <meta charset="UTF-8" />
            <title>arch -- Excalidraw</title>
            <style>
              html, body, #root { margin: 0; padding: 0; width: 100%; height: 100%; overflow: hidden; }
              #save-indicator {
                position: fixed; bottom: 12px; left: 50%; transform: translateX(-50%);
                background: rgba(0,0,0,0.7); color: #fff; padding: 4px 14px;
                border-radius: 6px; font: 13px monospace; opacity: 0;
                transition: opacity 0.3s; pointer-events: none; z-index: 9999;
              }
            </style>
            <script src="https://unpkg.com/react@18/umd/react.production.min.js"></script>
            <script src="https://unpkg.com/react-dom@18/umd/react-dom.production.min.js"></script>
            <script src="https://unpkg.com/@excalidraw/excalidraw@0.17.6/dist/excalidraw.production.min.js"></script>
          </head>
          <body>
            <div id="root"></div>
            <div id="save-indicator">saved</div>
            <script>
              var Excalidraw = ExcalidrawLib.Excalidraw;
              var excalidrawAPI = null;
              var indicator = document.getElementById('save-indicator');
              var indicatorTimer = null;

              function showIndicator(msg) {
                indicator.textContent = msg;
                indicator.style.opacity = '1';
                clearTimeout(indicatorTimer);
                indicatorTimer = setTimeout(function() { indicator.style.opacity = '0'; }, 1500);
              }

              var SAVE_APP_STATE_KEYS = [
                'gridSize', 'viewBackgroundColor', 'theme',
                'currentItemStrokeColor', 'currentItemBackgroundColor',
                'currentItemFillStyle', 'currentItemStrokeWidth',
                'currentItemStrokeStyle', 'currentItemRoughness',
                'currentItemOpacity', 'currentItemFontFamily',
                'currentItemFontSize', 'currentItemTextAlign',
                'currentItemStartArrowhead', 'currentItemEndArrowhead',
                'currentItemRoundness', 'exportBackground',
                'exportWithDarkMode', 'exportScale', 'exportEmbedScene',
                'name', 'zenModeEnabled', 'objectsSnapModeEnabled'
              ];

              function saveNow() {
                if (!excalidrawAPI) return;
                var elements = excalidrawAPI.getSceneElements();
                var fullState = excalidrawAPI.getAppState();
                var files     = excalidrawAPI.getFiles();
                var appState = {};
                SAVE_APP_STATE_KEYS.forEach(function(k) {
                  if (k in fullState) appState[k] = fullState[k];
                });
                fetch('/api/save', {
                  method: 'POST',
                  headers: { 'Content-Type': 'application/json' },
                  body: JSON.stringify({
                    type: 'excalidraw',
                    version: 2,
                    source: 'http://localhost:3741',
                    elements: elements,
                    appState: appState,
                    files: files || {}
                  })
                }).then(function() {
                  showIndicator('saved');
                }).catch(function() {
                  showIndicator('save failed');
                });
              }

              document.addEventListener('keydown', function(e) {
                if (e.altKey && e.key === 'w') {
                  e.preventDefault();
                  saveNow();
                }
              });

              setInterval(function() {
                fetch('/api/heartbeat', { method: 'POST' }).catch(function() {
                  document.getElementById('root').style.display = 'none';
                  document.body.style.background = '#1e1e1e';
                  document.body.style.color = '#aaa';
                  document.body.style.display = 'flex';
                  document.body.style.alignItems = 'center';
                  document.body.style.justifyContent = 'center';
                  document.body.style.font = '24px monospace';
                  document.body.textContent = 'server stopped';
                });
              }, 3000);

              fetch('/api/load')
                .then(function(r) { return r.json(); })
                .then(function(data) {
                  ReactDOM.render(
                    React.createElement(Excalidraw, {
                      initialData: {
                        elements: data.elements || [],
                        appState: data.appState || {},
                        files: data.files || {}
                      },
                      excalidrawAPI: function(api) { excalidrawAPI = api; }
                    }),
                    document.getElementById('root')
                  );
                });
            </script>
          </body>
          </html>
        '';

        excalidrawServer = pkgs.writeText "excalidraw-server.mjs" ''
          import { createServer } from 'http';
          import { readFileSync, writeFileSync } from 'fs';
          import { fileURLToPath } from 'url';
          import { dirname, join } from 'path';

          const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
          const ROOT = join(SCRIPT_DIR, '..');
          const FILE = join(ROOT, 'docs', 'arch.excalidraw');
          const HTML  = join(SCRIPT_DIR, 'excalidraw-app.html');
          const PORT  = 3741;
          const HEARTBEAT_TIMEOUT = 8000;
          let lastHeartbeat = Date.now();

          setInterval(function() {
            if (Date.now() - lastHeartbeat > HEARTBEAT_TIMEOUT) {
              console.log('Tab closed, shutting down.');
              process.exit(0);
            }
          }, 3000);

          const server = createServer(function(req, res) {
            const cors = { 'Access-Control-Allow-Origin': '*' };

            if (req.method === 'OPTIONS') {
              res.writeHead(204, cors);
              res.end();
              return;
            }

            if (req.method === 'GET' && req.url === '/') {
              res.writeHead(200, Object.assign({ 'Content-Type': 'text/html' }, cors));
              res.end(readFileSync(HTML));
              return;
            }

            if (req.method === 'POST' && req.url === '/api/heartbeat') {
              lastHeartbeat = Date.now();
              res.writeHead(204, cors);
              res.end();
              return;
            }

            if (req.method === 'GET' && req.url === '/api/load') {
              res.writeHead(200, Object.assign({ 'Content-Type': 'application/json' }, cors));
              res.end(readFileSync(FILE, 'utf8'));
              return;
            }

            if (req.method === 'POST' && req.url === '/api/save') {
              let body = "";
              req.on('data', function(chunk) { body += chunk; });
              req.on('end', function() {
                try {
                  const parsed = JSON.parse(body);
                  writeFileSync(FILE, JSON.stringify(parsed, null, 2));
                  res.writeHead(200, Object.assign({ 'Content-Type': 'application/json' }, cors));
                  res.end('{"ok":true}');
                  process.stdout.write('saved\n');
                } catch (e) {
                  res.writeHead(500, cors);
                  res.end(JSON.stringify({ error: e.message }));
                }
              });
              return;
            }

            res.writeHead(404, cors);
            res.end();
          });

          server.listen(PORT, function() {
            console.log('Excalidraw ready: http://localhost:' + PORT);
            console.log('Editing: ' + FILE);
            console.log('Alt+W to save. Ctrl+C to stop.');
          });
        '';

        exToMdScript = pkgs.writeText "ex-to-md.mjs" ''
          import { readFileSync, writeFileSync } from 'fs';
          import { fileURLToPath } from 'url';
          import { dirname, join } from 'path';

          const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
          const IN   = join(ROOT, 'docs', 'arch.excalidraw');
          const OUT  = join(ROOT, 'docs', 'arch.md');

          const data = JSON.parse(readFileSync(IN, 'utf8'));
          const els  = data.elements.filter(function(e) { return !e.isDeleted; });

          const rects  = els.filter(function(e) { return e.type === 'rectangle' || e.type === 'frame'; });
          const texts  = els.filter(function(e) { return e.type === 'text'; });
          const arrows = els.filter(function(e) { return e.type === 'arrow'; });

          function contains(rect, text) {
            const PAD = 80;
            return (
              text.x >= rect.x - PAD &&
              text.y >= rect.y - PAD &&
              text.x + text.width  <= rect.x + rect.width  + PAD &&
              text.y + text.height <= rect.y + rect.height + PAD
            );
          }

          function makeId(str) {
            return str
              .replace(/^#+\s*/, "")
              .replace(/[^a-zA-Z0-9]/g, "_")
              .replace(/_+/g, "_")
              .replace(/^_|_$/g, "");
          }

          function escapeMermaid(str) {
            return str
              .replace(/"/g, '#quot;')
              .replace(/</g, '#lt;')
              .replace(/>/g, '#gt;')
              .replace(/\n/g, '<br/>');
          }

          var nodes = [];
          var assignedTextIds = new Set();

          if (rects.length > 0) {
            rects.forEach(function(rect) {
              var inside  = texts.filter(function(t) { return contains(rect, t); });
              var heading = inside.find(function(t) { return /^#+\s/.test(t.text); });
              var body    = inside.filter(function(t) { return !(/^#+\s/.test(t.text)); });
              inside.forEach(function(t) { assignedTextIds.add(t.id); });

              var rawLabel = heading ? heading.text.replace(/^#+\s*/, "") : ("node_" + rect.id.slice(0, 6));
              var id       = makeId(rawLabel);
              var desc     = body.map(function(t) { return t.text; }).join('\n');
              var label    = desc ? (rawLabel + '<br/><br/>' + escapeMermaid(desc)) : escapeMermaid(rawLabel);
              nodes.push({ id: id, label: label, rectId: rect.id });
            });

            texts.forEach(function(t) {
              if (!assignedTextIds.has(t.id)) {
                var id = makeId(t.text.split('\n')[0]);
                if (id) nodes.push({ id: id, label: escapeMermaid(t.text), rectId: null });
              }
            });
          } else {
            var headings  = texts.filter(function(t) { return /^#+\s/.test(t.text); });
            var bodyTexts = texts.filter(function(t) { return !(/^#+\s/.test(t.text)); });
            headings.forEach(function(h) {
              var rawLabel = h.text.replace(/^#+\s*/, "");
              var id  = makeId(rawLabel);
              var desc = bodyTexts
                .filter(function(b) { return Math.abs(b.x - h.x) < 300; })
                .map(function(b) { return b.text; })
                .join('\n');
              var label = desc ? (rawLabel + '<br/><br/>' + escapeMermaid(desc)) : escapeMermaid(rawLabel);
              nodes.push({ id: id, label: label, rectId: null });
            });
          }

          nodes.sort(function(a, b) {
            var ra = rects.find(function(r) { return r.id === a.rectId; });
            var rb = rects.find(function(r) { return r.id === b.rectId; });
            return ((ra && ra.x) || 0) - ((rb && rb.x) || 0);
          });

          var edges = [];
          arrows.forEach(function(arrow) {
            var startId = arrow.startBinding && arrow.startBinding.elementId;
            var endId   = arrow.endBinding   && arrow.endBinding.elementId;
            if (!startId || !endId) return;
            var from = nodes.find(function(n) { return n.rectId === startId; });
            var to   = nodes.find(function(n) { return n.rectId === endId;   });
            if (!from || !to) return;
            var edgeLabel = "";
            if (arrow.boundElements) {
              var labelEl = els.find(function(e) {
                return e.type === 'text' && arrow.boundElements.some(function(b) { return b.id === e.id; });
              });
              if (labelEl) edgeLabel = escapeMermaid(labelEl.text);
            }
            edges.push({ from: from.id, to: to.id, label: edgeLabel });
          });

          var lines = ['```mermaid', 'flowchart LR'];
          nodes.forEach(function(n) {
            lines.push('  ' + n.id + '["' + n.label + '"]');
          });
          edges.forEach(function(e) {
            if (e.label) {
              lines.push('  ' + e.from + ' -->|"' + e.label + '"| ' + e.to);
            } else {
              lines.push('  ' + e.from + ' --> ' + e.to);
            }
          });
          lines.push('```');

          writeFileSync(OUT, lines.join('\n') + '\n');
          console.log('Written: ' + OUT);
        '';

        mdToExScript = pkgs.writeText "md-to-ex.mjs" ''
          import { readFileSync, writeFileSync } from 'fs';
          import { fileURLToPath } from 'url';
          import { dirname, join } from 'path';
          import { JSDOM } from 'jsdom';

          // Set up DOM globals before loading mermaid (needs DOMPurify + SVG layout)
          const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
          global.window   = dom.window;
          global.document = dom.window.document;
          global.DOMParser = dom.window.DOMParser;
          Object.defineProperty(global, 'navigator', { value: dom.window.navigator, writable: true, configurable: true });
          if (dom.window.SVGElement) {
            dom.window.SVGElement.prototype.getBBox = function() {
              return { x: 0, y: 0, width: 100, height: 50 };
            };
          }

          // Dynamic import AFTER globals are set
          const { parseMermaidToExcalidraw } = await import('@excalidraw/mermaid-to-excalidraw');

          const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
          const IN   = join(ROOT, 'docs', 'arch.md');
          const OUT  = join(ROOT, 'docs', 'arch.excalidraw');

          const src = readFileSync(IN, 'utf8');
          const match = src.match(/```mermaid\n([\s\S]*?)```/);
          if (!match) {
            console.error('No mermaid code block found in ' + IN);
            process.exit(1);
          }

          const mermaidSrc = match[1].trim();
          console.log('Parsing mermaid...');

          const result = await parseMermaidToExcalidraw(mermaidSrc);

          const excalidraw = {
            type: 'excalidraw',
            version: 2,
            source: 'mermaid-to-excalidraw',
            elements: result.elements || [],
            appState: {
              gridSize: null,
              gridStep: 5,
              viewBackgroundColor: '#ffffff'
            },
            files: result.files || {}
          };

          writeFileSync(OUT, JSON.stringify(excalidraw, null, 2));
          console.log('Written: ' + OUT);
        '';

        exCmd = pkgs.writeShellScriptBin "ex" ''
          BROWSER=""
          while [[ $# -gt 0 ]]; do
            case "$1" in
              -b|--browser) BROWSER="$2"; shift 2 ;;
              *) shift ;;
            esac
          done
          cd "$(git rev-parse --show-toplevel)"
          node _scripts/excalidraw-server.mjs &
          SERVER_PID=$!
          sleep 0.5
          if [[ -n "$BROWSER" ]]; then
            "$BROWSER" "http://localhost:3741"
          else
            ${pkgs.xdg-utils}/bin/xdg-open "http://localhost:3741"
          fi
          wait $SERVER_PID
        '';

        exToMdCmd = pkgs.writeShellScriptBin "ex-to-md" ''
          cd "$(git rev-parse --show-toplevel)"
          node _scripts/ex-to-md.mjs
        '';

        mdToExCmd = pkgs.writeShellScriptBin "md-to-ex" ''
          cd "$(git rev-parse --show-toplevel)"
          node _scripts/md-to-ex.mjs
        '';

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

              # Excalidraw tools: write scripts from nix store and install npm deps
              mkdir -p _scripts
              cp -f ${excalidrawPackageJson}  _scripts/package.json
              cp -f ${excalidrawAppHtml}      _scripts/excalidraw-app.html
              cp -f ${excalidrawServer}       _scripts/excalidraw-server.mjs
              cp -f ${exToMdScript}           _scripts/ex-to-md.mjs
              cp -f ${mdToExScript}           _scripts/md-to-ex.mjs
              chmod +w _scripts/package.json _scripts/excalidraw-app.html _scripts/excalidraw-server.mjs _scripts/ex-to-md.mjs _scripts/md-to-ex.mjs
              if [ ! -d "$PWD/_scripts/node_modules/@excalidraw" ]; then
                echo "excalidraw tools: npm install..."
                ${pkgs.nodejs}/bin/npm install --prefix "$PWD/_scripts" 2>&1 | tail -3
              fi
            '';

          env = {
            RUST_BACKTRACE = 1;
            RUST_LIB_BACKTRACE = 0;
          };

          packages = [
            mold
            nodejs
            openssl
            pkg-config
            rust
            xdg-utils
            exCmd
            exToMdCmd
            mdToExCmd
          ] ++ pre-commit-check.enabledPackages ++ github.enabledPackages ++ rs.enabledPackages;
        };
      }
    );
}
