import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { checkoutRepository, mieruFeatures, transportFeatures, visionSupported } from './client-capability-evidence.mjs';

const fixture = name => readFileSync(new URL(`./fixtures/client-capabilities/${name}.go.txt`, import.meta.url), 'utf8');

test('legacy Trojan if dispatch proves WS and gRPC despite a separate TCP switch', () => {
  assert.deepEqual(transportFeatures(fixture('trojan-if-dispatch'), 'trojan'), {
    'trojan.ws': true, 'trojan.grpc': true,
  });
});

test('network switch proves implemented branches without inferring absent transports', () => {
  assert.deepEqual(transportFeatures(fixture('vless-switch-dispatch'), 'vless'), {
    'vless.ws': true, 'vless.grpc': true, 'vless.h2': true, 'vless.http': true,
  });
  assert.deepEqual(transportFeatures('switch options.Network { case "xhttp": return errors.New("unsupported") }', 'vless'), {});
  assert.deepEqual(transportFeatures('// case "ws": StreamWebsocketConn()\nif options.Network == "ws" { return errors.New("unsupported") }', 'trojan'), {});
  // Newer adapters dispatch through dedicated clients instead of StreamGun.
  // https://github.com/chen08209/Clash.Meta/blob/0f7f05adff5e2c49775a112dcfe05a6aa36fda0c/adapter/outbound/vless.go
  assert.deepEqual(transportFeatures('switch proxy.option.Network { case "grpc": return proxy.gunClient.Dial(); case "xhttp": return proxy.xhttpClient.Dial(ctx) }', 'vless'), {
    'vless.grpc': true, 'vless.xhttp': true,
  });
  assert.deepEqual(transportFeatures('switch proxy.option.Network { case "grpc": return proxy.gunTransport.Dial() }', 'trojan'), {
    'trojan.grpc': true,
  });
});

test('comments, unrelated switches and string contents do not prove network support', () => {
  assert.deepEqual(transportFeatures('switch options.Other { case "ws": StreamWebsocketConn() }', 'trojan'), {});
  assert.deepEqual(transportFeatures('const explanation = `switch options.Network { case "ws": StreamWebsocketConn() }`', 'trojan'), {});
});

test('Vision requires both actual constant use and its verified definition', () => {
  const source = fixture('vless-switch-dispatch');
  assert.equal(visionSupported(source, 'const XRV = "xtls-rprx-vision"'), true);
  assert.equal(visionSupported(source, ''), false);
  assert.equal(visionSupported(source, 'const XRV = "different-flow"'), false);
  assert.equal(visionSupported('// vless.XRV', 'const XRV = "xtls-rprx-vision"'), false);
});

test('Mieru negatives require explicit rejection or disabled relay, not missing words', () => {
  assert.deepEqual(mieruFeatures('if option.Transport != "TCP" { return fmt.Errorf("unsupported") }; Base{ udp: false }'), {
    'mieru.udp-transport': false, 'mieru.udp-relay': false,
  });
  assert.deepEqual(mieruFeatures('var options UnrecognizedSourceShape'), {});
  assert.deepEqual(mieruFeatures('if option.Transport != "TCP" && option.Transport != "UDP" { return fmt.Errorf("unsupported") }'), {});
  assert.deepEqual(mieruFeatures('TransportProtocol_UDP.Enum(); Base{ udp: option.UDP }; config.Level = option.Multiplexing'), {
    'mieru.udp-transport': true, 'mieru.udp-relay': true, 'mieru.multiplexing': true,
  });
  assert.deepEqual(mieruFeatures('BaseOptions{ UDP: option.UDP }'), { 'mieru.udp-relay': true });
});

test('a second cached checkout fetches a newly published release tag', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'client-evidence-test-'));
  const source = path.join(directory, 'public-source');
  const cached = path.join(directory, 'cached.git');
  const git = (...args) => execFileSync('git', ['-c', 'user.name=Example Maintainer',
    '-c', 'user.email=maintainer@example.invalid', ...args], {
    cwd: source, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
  try {
    mkdirSync(source);
    git('init');
    writeFileSync(path.join(source, 'version.txt'), 'one\n');
    git('add', 'version.txt');
    git('commit', '-m', 'First synthetic release');
    git('tag', 'v1.0.0');
    checkoutRepository(cached, source);
    writeFileSync(path.join(source, 'version.txt'), 'two\n');
    git('commit', '-am', 'Second synthetic release');
    git('tag', 'v1.1.0');
    assert.equal(git(`--git-dir=${cached}`, 'tag', '--list', 'v1.1.0'), '');
    checkoutRepository(cached, source);
    assert.equal(git(`--git-dir=${cached}`, 'show', 'v1.1.0:version.txt'), 'two');
    assert.equal(git(`--git-dir=${cached}`, 'show', 'v1.0.0:version.txt'), 'one');
  } finally {
    // Verify the recursive-delete boundary even if the test failed mid-checkout.
    assert.ok(realpathSync(directory).startsWith(realpathSync(tmpdir()) + path.sep));
    rmSync(directory, { recursive: true, force: true });
  }
});
