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
