import { test } from "node:test";
import assert from "node:assert/strict";
import { PairingError } from "../src/errors.ts";
import {
  PairLink,
  credentialKey,
  redeemPairLink,
} from "../src/pairing.ts";
import type { PinnedTransport } from "../src/transport.ts";
import {
  MemoryStore,
  fakeTransport,
  fixture,
  makeUuid,
  rejectsErr,
  throwsErr,
  type TransportCall,
} from "./fakes.ts";

const VALID =
  "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=9999999999999#code=0123ABCDEF45";
const CODE = "0123ABCDEF45";

test("pairing link parses into its envelope fields and redacts the code", () => {
  const link = PairLink.parse(`  ${VALID}  `);
  assert.equal(link.host, "100.64.0.10");
  assert.equal(link.port, 47329);
  assert.equal(link.serverId, "00000000-0000-4000-8000-00000000aa01");
  assert.equal(link.expiresAt, 9999999999999n);
  assert.equal(link.expired(0), false);
  const text = JSON.stringify(link);
  assert.ok(!text.includes(CODE), "serialized link leaked the code");
  assert.match(text, /<redacted>/);
});

test("tailnet IPv6 host parses and canonicalizes", () => {
  const link = PairLink.parse(
    "shina://pair/v1?host=fd7a:115c:a1e0:0000:0000:0000:0000:0012&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=9999999999999#code=0123ABCDEF45",
  );
  assert.equal(link.host, "fd7a:115c:a1e0::12");
});

test("magicdns host names are accepted", () => {
  const link = PairLink.parse(
    "shina://pair/v1?host=studio.tail74ed91.ts.net&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=9999999999999#code=0123ABCDEF45",
  );
  assert.equal(link.host, "studio.tail74ed91.ts.net");
});

const REJECTED: Array<[string, string]> = [
  ["wrong scheme", "shina://join/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["wrong version", "shina://pair/v2?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["missing port", "shina://pair/v1?host=100.64.0.10&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["empty host", "shina://pair/v1?host=&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["unknown field", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1&admin=1#code=0123ABCDEF45"],
  ["duplicate field", "shina://pair/v1?host=100.64.0.10&port=47329&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["percent escape", "shina://pair/v1?host=100%2E64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["embedded space", "shina://pair/v1?host=100.64.0.10&port=47 329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["no fragment", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1"],
  ["no code", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#"],
  ["short code", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF4"],
  ["lowercase code", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123abcdef45"],
  ["port zero", "shina://pair/v1?host=100.64.0.10&port=0&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["port too big", "shina://pair/v1?host=100.64.0.10&port=65536&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["port leading zero", "shina://pair/v1?host=100.64.0.10&port=047329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["uppercase uuid", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000AA01&expiresAt=1#code=0123ABCDEF45"],
  ["unhyphenated uuid", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=0000000000004000800000000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["expiry zero", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=0#code=0123ABCDEF45"],
  ["expiry leading zero", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=01#code=0123ABCDEF45"],
  ["expiry overflows u64", "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=18446744073709551616#code=0123ABCDEF45"],
  ["public ip", "shina://pair/v1?host=8.8.8.8&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["lan ip", "shina://pair/v1?host=192.168.1.5&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["loopback", "shina://pair/v1?host=127.0.0.1&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["wildcard", "shina://pair/v1?host=0.0.0.0&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["cg nat edge low", "shina://pair/v1?host=100.63.255.255&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["cg nat edge high", "shina://pair/v1?host=100.128.0.0&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["non tailnet v6", "shina://pair/v1?host=2606:4700:4700::1111&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["v6 loopback", "shina://pair/v1?host=::1&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["bad dns label", "shina://pair/v1?host=-bad.ts.net&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
  ["userinfo style host", "shina://pair/v1?host=user@100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45"],
];

for (const [name, link] of REJECTED) {
  test(`link rejected: ${name}`, () => {
    assert.throws(() => PairLink.parse(link), PairingError);
  });
}

test("code inside the query is a specific error and never echoed", () => {
  const err = throwsErr(
    () =>
      PairLink.parse(
        `shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1&code=${CODE}#code=0123ABCDEF45`,
      ),
    PairingError,
  );
  assert.match(err.message, /fragment/);
  assert.ok(!err.message.includes(CODE));
});

test("oversized link rejected", () => {
  const long = `shina://pair/v1?host=${"a".repeat(64)}.ts.net&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1#code=0123ABCDEF45`;
  assert.throws(() => PairLink.parse(long + "x".repeat(512)), PairingError);
});

function redeemAdapters(
  transport: { transport: PinnedTransport; calls?: TransportCall[] },
  extra: object = {},
) {
  return {
    transport: transport.transport,
    store: new MemoryStore(),
    uuid: makeUuid(),
    now: () => 1000,
    ...extra,
  };
}

test("redeem verifies identity before the code is sent", async () => {
  const { transport, calls } = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
    "POST /v1/pair": (request) => {
      assert.deepEqual(request.json, {
        deviceId: "00000000-0000-4000-8000-000000000001",
        code: CODE,
        expectedServerId: fixture.serverId,
      });
      return {
        status: 200,
        json: { serverId: fixture.serverId, token: "tok_issued", peer: false },
      };
    },
  });
  const adapters = redeemAdapters({ transport, calls });
  const link = PairLink.parse(VALID);
  const cred = await redeemPairLink(link, adapters);
  assert.equal(cred.serverId, fixture.serverId);
  assert.equal(cred.token, "tok_issued");
  assert.equal(cred.deviceId, "00000000-0000-4000-8000-000000000001");
  assert.equal(cred.host, "100.64.0.10");
  assert.equal(cred.port, 47329);
  assert.deepEqual(
    calls.map((call) => `${call.request.method} ${call.request.path}`),
    ["GET /v1/identity", "POST /v1/pair"],
    "identity must be fetched before the code is posted",
  );
  const stored = await adapters.store.get(credentialKey(fixture.serverId));
  assert.ok(stored);
  assert.equal(JSON.parse(stored as string).token, "tok_issued");
});

test("redeem refuses to send the code when identity mismatches", async () => {
  const { transport, calls } = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: "00000000-0000-4000-8000-ffffffffffff" } },
  });
  const link = PairLink.parse(VALID);
  const err = await rejectsErr(
    redeemPairLink(link, redeemAdapters({ transport, calls })),
    PairingError,
  );
  assert.ok(!err.message.includes(CODE));
  assert.equal(
    calls.filter((call) => call.request.method === "POST").length,
    0,
    "the pairing code was sent to an unverified host",
  );
});

test("redeem refuses redirects and never posts the code", async () => {
  const { transport, calls } = fakeTransport({
    "GET /v1/identity": {
      status: 302,
      headers: { location: "http://evil.example/" },
    },
  });
  const err = await rejectsErr(
    redeemPairLink(PairLink.parse(VALID), redeemAdapters({ transport, calls })),
    PairingError,
  );
  assert.match(err.message, /redirect/i);
  assert.ok(!err.message.includes(CODE));
  assert.equal(calls.filter((call) => call.request.method === "POST").length, 0);
});

test("redeem surfaces pair refusals without leaking the code", async () => {
  for (const status of [400, 403, 409, 500]) {
    const { transport, calls } = fakeTransport({
      "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
      "POST /v1/pair": { status, json: { error: { message: "no" } } },
    });
    const err = await rejectsErr(
      redeemPairLink(PairLink.parse(VALID), redeemAdapters({ transport, calls })),
      PairingError,
    );
    assert.ok(!err.message.includes(CODE));
  }
});

test("redeem validates the issued identity and token", async () => {
  const bad = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
    "POST /v1/pair": {
      status: 200,
      json: { serverId: "00000000-0000-4000-8000-ffffffffffff", token: "t" },
    },
  });
  await assert.rejects(
    redeemPairLink(PairLink.parse(VALID), redeemAdapters(bad)),
    /identity did not match/,
  );
  const noToken = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
    "POST /v1/pair": { status: 200, json: { serverId: fixture.serverId } },
  });
  await assert.rejects(
    redeemPairLink(PairLink.parse(VALID), redeemAdapters(noToken)),
    /did not issue a device token/,
  );
});

test("expired link fails before any transport call", async () => {
  const { transport, calls } = fakeTransport({});
  const link = PairLink.parse(
    "shina://pair/v1?host=100.64.0.10&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=1000#code=0123ABCDEF45",
  );
  await assert.rejects(
    redeemPairLink(link, redeemAdapters({ transport, calls })),
    /expired/,
  );
  assert.equal(calls.length, 0);
});

test("magicdns resolves through the adapter and requires tailnet answers", async () => {
  const named = PairLink.parse(
    "shina://pair/v1?host=studio.tail74ed91.ts.net&port=47329&serverId=00000000-0000-4000-8000-00000000aa01&expiresAt=9999999999999#code=0123ABCDEF45",
  );
  const { transport, calls } = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
    "POST /v1/pair": {
      status: 200,
      json: { serverId: fixture.serverId, token: "tok_issued" },
    },
  });
  const resolved: string[] = [];
  const resolve = async (host: string): Promise<readonly string[]> => {
    resolved.push(host);
    return ["100.64.0.10", "fd7a:115c:a1e0::10"];
  };
  const cred = await redeemPairLink(named, { ...redeemAdapters({ transport, calls }), resolve });
  assert.deepEqual(resolved, ["studio.tail74ed91.ts.net"]);
  assert.equal(calls[0]?.address.ip, "100.64.0.10");
  assert.equal(cred.host, "studio.tail74ed91.ts.net");

  const badResolve = async (): Promise<readonly string[]> => ["8.8.8.8"];
  await assert.rejects(
    redeemPairLink(named, {
      ...redeemAdapters({ transport, calls }),
      resolve: badResolve,
    }),
    /tailnet/,
  );
  await assert.rejects(
    redeemPairLink(named, redeemAdapters({ transport, calls })),
    /resolver/,
  );
});

test("missing adapters fail closed", async () => {
  const link = PairLink.parse(VALID);
  const { transport } = fakeTransport({});
  await assert.rejects(
    // @ts-expect-error intentionally missing transport
    redeemPairLink(link, { store: new MemoryStore(), uuid: makeUuid() }),
    /pinned transport/,
  );
  await assert.rejects(
    // @ts-expect-error intentionally missing store
    redeemPairLink(link, { transport, uuid: makeUuid() }),
    /secure credential store/,
  );
  await assert.rejects(
    // @ts-expect-error intentionally missing uuid
    redeemPairLink(link, { transport, store: new MemoryStore() }),
    /uuid/,
  );
});

test("code is never placed in a path or query", async () => {
  const { transport, calls } = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
    "POST /v1/pair": {
      status: 200,
      json: { serverId: fixture.serverId, token: "tok_issued" },
    },
  });
  await redeemPairLink(PairLink.parse(VALID), redeemAdapters({ transport, calls }));
  for (const call of calls) {
    assert.ok(!call.request.path.includes(CODE));
    assert.ok(!(call.request.path.includes("code")));
  }
});

test("credential round-trips through the store shape", async () => {
  const { transport } = fakeTransport({
    "GET /v1/identity": { status: 200, json: { serverId: fixture.serverId } },
    "POST /v1/pair": {
      status: 200,
      json: { serverId: fixture.serverId, token: "tok_issued" },
    },
  });
  const adapters = redeemAdapters({ transport });
  await redeemPairLink(PairLink.parse(VALID), adapters);
  const { loadCredential } = await import("../src/pairing.ts");
  const loaded = await loadCredential(adapters.store, fixture.serverId);
  assert.equal(loaded?.token, "tok_issued");
  assert.equal(loaded?.host, "100.64.0.10");
});
