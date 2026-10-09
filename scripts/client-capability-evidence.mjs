import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const git = (...args) => execFileSync('git', args, {
  encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
}).trim();

export function checkoutRepository(directory, remote) {
  if (!existsSync(directory)) {
    git('-c', 'credential.helper=', 'clone', '--bare', '--filter=blob:none',
      '--depth=1', '--no-single-branch', remote, directory);
  } else {
    // A cached checkout is not immutable: new release tags must be fetched.
    // Do not force moved tags; a publisher changing evidence needs review.
    git('-c', 'credential.helper=', `--git-dir=${directory}`, 'fetch',
      '--prune', '--prune-tags', '--tags', '--depth=1', remote);
  }
  return directory;
}

// A small Go lexer, used only to recognize reviewed implementation shapes.
// Comments and string contents cannot introduce fake braces or control flow.
function tokens(source) {
  return [...source.matchAll(/\/\*[\s\S]*?\*\/|\/\/[^\n]*|"(?:\\.|[^"\\])*"|`[^`]*`|'(?:\\.|[^'\\])*'|[A-Za-z_]\w*|==|!=|[^\s]/g)]
    .map(match => match[0]).filter(token => !token.startsWith('//') && !token.startsWith('/*'));
}

function closingBrace(code, open) {
  let depth = 1;
  for (let end = open + 1; end < code.length; end++) {
    if (code[end] === '{') depth++;
    if (code[end] === '}' && --depth === 0) return end;
  }
  return -1;
}

const implementation = {
  ws: /\bStreamWebsocketConn\b/,
  grpc: /\b(?:StreamGun\w*|NewHTTP2Client)\b|\b(?:gunClient|gunTransport)\s*\.\s*Dial\b/,
  h2: /\bStreamH2Conn\b/,
  http: /\bStreamHTTPConn\b/,
  xhttp: /\bxhttpClient\s*\.\s*Dial\b/,
};

export function transportFeatures(source, protocol) {
  const code = tokens(source);
  const features = {};
  const record = (names, body) => {
    for (const name of names) {
      if (implementation[name]?.test(body.join(' '))) features[`${protocol}.${name}`] = true;
    }
  };
  for (let at = 0; at < code.length; at++) {
    if (code[at] !== 'switch' && code[at] !== 'if') continue;
    const open = code.indexOf('{', at + 1);
    if (open < 0) continue;
    const expression = code.slice(at + 1, open).join('');
    const end = closingBrace(code, open);
    if (end < 0) continue;
    if (code[at] === 'if') {
      const match = expression.match(/^(?:\w+\.)+Network=="([a-z0-9]+)"$/);
      if (match) record([match[1]], code.slice(open + 1, end));
    } else if (/^(?:\w+\.)+Network$/.test(expression)) {
      let depth = 0;
      let names = [];
      let start = open + 1;
      for (let cursor = open + 1; cursor <= end; cursor++) {
        if (depth === 0 && (code[cursor] === 'case' || code[cursor] === 'default' || cursor === end)) {
          record(names, code.slice(start, cursor));
          names = [];
          if (cursor === end) break;
          while (cursor < end && code[cursor] !== ':') {
            if (/^"[a-z0-9]+"$/.test(code[cursor])) names.push(code[cursor].slice(1, -1));
            cursor++;
          }
          start = cursor + 1;
        } else if (code[cursor] === '{') depth++;
        else if (code[cursor] === '}') depth--;
      }
    }
  }
  // An absent recognized branch proves nothing: dispatch may live elsewhere.
  return features;
}

export function mieruFeatures(source) {
  const code = tokens(source).join(' ');
  const features = {};
  if (/TransportProtocol_UDP/.test(code)) features['mieru.udp-transport'] = true;
  else if (/if option \. Transport != "TCP" \{ return (?:fmt \. Errorf|errors \. New)/.test(code)) {
    features['mieru.udp-transport'] = false;
  }
  if (/\b(?:udp|UDP) : option \. UDP/.test(code)) features['mieru.udp-relay'] = true;
  else if (/\b(?:udp|UDP) : false/.test(code)) features['mieru.udp-relay'] = false;
  for (const [feature, field] of Object.entries({
    multiplexing: 'Multiplexing', 'handshake-mode': 'HandshakeMode', 'traffic-pattern': 'TrafficPattern',
  })) {
    if (new RegExp(`\\boption \\. ${field}\\b`).test(code)) features[`mieru.${feature}`] = true;
  }
  return features;
}

export function visionSupported(adapterSource, vlessSource) {
  const adapter = tokens(adapterSource).join(' ');
  const constants = tokens(vlessSource).join(' ');
  return /\bvless \. XRV\b/.test(adapter)
    && /\bXRV = "xtls-rprx-vision"/.test(constants);
}
