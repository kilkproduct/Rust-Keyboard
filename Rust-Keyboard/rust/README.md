# Rust core (`rust/`)

This directory holds the dictionary and suggestion engine. It replaced the C++
engine that used to live in `native/jni/src/dictionary` and
`native/jni/src/suggest`; that code, and the voice-input natives that shared its
build, are not in this tree any more.

```
Kotlin / Java (unchanged API)
        │  System.loadLibrary("keyboard_core")  ← the only engine
        ▼
rust/jni  ── JNI bridge, registers the natives of the dictionary classes
        ▼
rust/core ── kb-core: dictionary reader, trie, personalisation log, proximity,
             typing / gesture search, scoring
```

`JNI_OnLoad` calls `RegisterNatives` for `BinaryDictionary`, `ProximityInfo`,
`DicTraverseSession` and `BinaryDictionaryUtils`. There is no second engine to
fall back to: `rust/build-android.sh` fails when cargo or the NDK is missing,
and `checkRustNative` refuses to package a release APK whose
`java/jniLibs/<abi>/libkeyboard_core.so` is absent.

## Modules

| File | Ports | Notes |
| --- | --- | --- |
| `core/src/bytes.rs` | `utils/byte_array_utils`, `format_utils` | panic-free big-endian readers, code point table decoding, `sint24` |
| `core/src/mmap.rs` | `dictionary/utils/buffer_with_extendable_buffer`, file reading | dictionaries are mapped read-only; pages are evicted by the kernel under pressure instead of living on the Java heap |
| `core/src/header.rs` | `dictionary/structure/v4/content/…`, `header_policy`, `header_read_write_utils` | 402/403 headers, attribute block |
| `core/src/ptrie.rs` | `structure/v4/content/ver4_patricia_trie_*`, `pt_common/*` | nodes are decoded on the fly from the mapped bytes |
| `core/src/triemap.rs` | `dictionary/utils/trie_map` | HAMT int → (terminal link, value) |
| `core/src/dict.rs` | `structure/v4/…`, `ver4_dict_constants` | whole-dictionary reader: trie walk, terminal table, language model, shortcuts |
| `core/src/user.rs` | `ExpandableBinaryDictionary` + `suggest/policyimpl/dictionary/dynamic_patricia_trie_*` | personalisation log: fixed-size records, in-place updates, oldest evicted first |
| `core/src/charutils.rs` | `utils/char_utils` | case / accent folding tables |
| `core/src/proximity.rs` | `suggest/core/dictionary/proximity_info`, `proximity_info_state_utils` | key grid + per-key distances, computed once per layout |
| `core/src/suggest.rs` | `suggest/core/{dic_node,session,policy,dictionary}`, `policyimpl/{typing,gesture}` | traversal, error costs, scoring, result formatting |
| `jni/src/env.rs` | `jni_common` | typed accessors over the JNI function table |
| `jni/src/lib.rs` | `org_futo_inputmethod_latin_BinaryDictionary.cpp` etc. | the native entry points; handles are boxed Rust values |

## Why it uses less RAM

* **Dictionaries are never copied.** The C++ engine read the dictionary into a
  heap buffer; here the file is `mmap`ed and every node is decoded from the
  mapping. Idle dictionaries cost a file mapping, not tens of megabytes of
  Java-heap-adjacent native memory.
* **No per-keystroke allocation.** Input points, code points, previous words and
  the result array live in thread-local scratch buffers that are reused. A
  keystroke in steady state performs zero heap allocations.
* **Fixed-size result slots.** `SuggestedWord` is a 48-code-point array plus
  four scalars, stored in one reusable `Vec`, matching the 40-result contract of
  the Java API.
* **The personalisation log is a flat array of records**, updated in place, with
  the oldest record evicted when full, instead of a rewritten Patricia trie.
* **No priority queue, no per-node heap nodes.** Traversal breadth is bounded by
  a fixed-size state array.
* **No network, no allocator, no dependencies**: `kb-core` links only `std`.

## Building

```sh
./rust/build-android.sh            # arm64-v8a (the shipped ABI) into java/jniLibs/
KB_ABIS="arm64-v8a,armeabi-v7a" ./rust/build-android.sh
```

`./gradlew assembleDebug` runs that script automatically through the
`rustNativeBuild` task (`-PskipRust` to reuse a library built earlier). The
script fails, with the reason, when cargo, the NDK or the requested target is
missing; release packaging additionally checks that the library exists.

Host-side checks:

```sh
cd rust
cargo test -p kb-core                     # unit tests (no Android needed)
cargo run -p kb-core --example probe -- /path/to/en_wordlist.combined.dict hello help
```

`probe` prints the header, walks every word with a time measurement and resolves
the words you pass, which is how you validate the reader against real
dictionaries.

## Status and known divergences

Implemented: v4 (402/403) reading, V2 static reading, header/attributes,
terminal table lookups with back-off, language-model n-gram probabilities,
shortcuts, not-a-word/offensive/blacklisted filtering, personalisation (unigram
+ n-gram contexts, flush, GC), proximity grids, typing search (match, proximity,
accent, case, substitution, omission, insertion, completion), gesture search,
predictions from the previous words, result ordering, scoring and output
packing.

Deliberate differences from the C++ engine, all documented at the top of
`suggest.rs`:

* traversal uses a fixed-size state array instead of a priority queue;
* multi-word suggestions come from committed previous words rather than from
  the full n-gram lattice;
* digraph expansion is not implemented;
* the gesture decoder is a simplified subsequence matcher (skip cost + unused
  point cost) rather than the full combined typing/gesture policy;
* `updateEntriesForInputEventsNative` (test-only Sketch API) is a no-op;
* `getITrieHandleNative` (unused letter-trie experiment) returns 0.

Not yet ported: dictionary creation/`dicttool` writing paths, and migration from
version 2/3 files into version 4. Neither is reachable from typing: the bundled
and downloaded dictionaries are already version 4.

The core is compiled and tested by CI on every push (`cargo test -p kb-core`,
plus a desktop-JVM harness that drives the JNI entry points against the real
English dictionary).
