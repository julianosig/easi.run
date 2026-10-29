// What the upload and drop fixtures list for a file they receive: "HOW NAME SIZE FNV1A", FNV1A being the 32-bit FNV-1a
// hash of its bytes, in hex (tests/tests/files.rs expects the same).
export async function receipt(file, how) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let hash = 0x811c9dc5;
  for (const b of bytes) hash = Math.imul(hash ^ b, 0x01000193) >>> 0;
  return `${how} ${file.name} ${bytes.length} ${hash.toString(16).padStart(8, '0')}`;
}
