// Mock key material. Real generation, parsing and encryption move to detour-core (ssh-key crate).

import { tr } from "../i18n";
import type { SshKey } from "./types";

const b64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const randomB64 = (n: number) => Array.from({ length: n }, () => b64[Math.floor(Math.random() * 64)]).join("");

export function fakePrivateKey(comment: string): string {
  const body = Array.from({ length: 6 }, () => randomB64(70)).join("\n");
  return `-----BEGIN OPENSSH PRIVATE KEY-----\n${body}\n-----END OPENSSH PRIVATE KEY-----\n# ${comment}\n`;
}

export function fakeKeyMaterial(algorithm: SshKey["algorithm"], comment: string) {
  return {
    fingerprint: `SHA256:${randomB64(7)}…${randomB64(4)}`,
    publicKey: `${algorithm === "ed25519" ? "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI" : "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQ"}${randomB64(43)} ${comment}`,
    privateKey: fakePrivateKey(comment),
  };
}

export type ParsedKey = { ok: true; algorithm: "ed25519" | "rsa" } | { ok: false; error: string };

/** Recognises the private key formats ssh-keygen and PuTTYgen (OpenSSH export) produce. */
export function parsePrivateKey(text: string): ParsedKey {
  const t = text.trim();
  if (!t) return { ok: false, error: tr().common.errors.key.empty };
  if (/^(ssh-(ed25519|rsa)|ecdsa-sha2)/.test(t)) return { ok: false, error: tr().common.errors.key.public };
  if (/PuTTY-User-Key-File/.test(t)) return { ok: false, error: tr().common.errors.key.putty };
  const m = t.match(/-----BEGIN (OPENSSH|RSA|EC) PRIVATE KEY-----[\s\S]+-----END \1 PRIVATE KEY-----/);
  if (!m) return { ok: false, error: tr().common.errors.key.notKey };
  return { ok: true, algorithm: m[1] === "RSA" ? "rsa" : "ed25519" };
}

/** Name for an imported key: the file name or the comment after the key block. */
export function suggestKeyName(text: string, fileName?: string): string {
  if (fileName) return fileName.replace(/\.(pem|key)$/i, "");
  return text.match(/#\s*(\S+)\s*$/)?.[1] ?? "imported-key";
}
