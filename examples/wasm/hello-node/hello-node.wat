;; SPDX-FileCopyrightText: 2026 Livspeil contributors
;; SPDX-License-Identifier: CC0-1.0

;; hello-node: a Livspeil wasm node package (core-module ABI version 1),
;; hand-written in WebAssembly text. Two node types:
;;
;;   demo.wave.sine@1   out = amplitude * sin(2 pi (frequency * time + phase))
;;                      (time-dependent; phase in cycles)
;;   demo.text.shout@1  out = text with ASCII letters upper-cased, plus "!";
;;                      an empty text is an invalid-input error on "text"
;;
;; Build: `cargo xtask wasm-example` assembles this file into
;; hello-node.wasm (the committed test fixture). See docs/wasm-nodes.md for
;; the ABI. The module imports nothing, as the host requires.

(module
  (memory (export "memory") 1 256)

  ;; ---- describe() buffer, offset 0, 331 bytes ---------------------------
  ;; All integers little-endian; strings are u32 length + UTF-8.
  (data (i32.const 0)
    "LSND" "\01\00\00\00"                 ;; magic, ABI version 1
    "\02\00\00\00"                        ;; 2 node types

    ;; node 0
    "\0e\00\00\00" "demo.wave.sine"       ;; id
    "\01\00\00\00"                        ;; version 1
    "\4e\00\00\00" "A sine wave: amplitude * sin(2 pi (frequency * time + phase)); phase in cycles"
    "\01"                                 ;; time_dependent = true
    "\03\00\00\00"                        ;; 3 inputs
    "\09\00\00\00" "frequency" "\00"      ;;   name, type Float
    "\01" "\00" "\00\00\00\00\00\00\f0\3f" ;;   default: some(Float 1.0)
    "\09\00\00\00" "amplitude" "\00"
    "\01" "\00" "\00\00\00\00\00\00\f0\3f" ;;   default 1.0
    "\05\00\00\00" "phase" "\00"
    "\01" "\00" "\00\00\00\00\00\00\00\00" ;;   default 0.0
    "\01\00\00\00"                        ;; 1 output
    "\03\00\00\00" "out" "\00" "\00"      ;;   out: Float, no default

    ;; node 1
    "\0f\00\00\00" "demo.text.shout"
    "\01\00\00\00"
    "\40\00\00\00" "Upper-case the ASCII letters of text and add an exclamation mark"
    "\00"                                 ;; time_dependent = false
    "\01\00\00\00"                        ;; 1 input
    "\04\00\00\00" "text" "\07"           ;;   type String
    "\01" "\07" "\05\00\00\00" "hello"    ;;   default: some(String "hello")
    "\01\00\00\00"
    "\03\00\00\00" "out" "\07" "\00")     ;;   out: String

  ;; ---- constant cook results ---------------------------------------------
  ;; err(invalid-input { port: "text", reason: "text is empty" }), 27 bytes
  (data (i32.const 512)
    "\01" "\00" "\04\00\00\00" "text" "\0d\00\00\00" "text is empty")
  ;; err(failed("unknown node index")), 24 bytes
  (data (i32.const 576)
    "\01" "\01" "\12\00\00\00" "unknown node index")

  ;; Bump allocator. Every call runs in a fresh instance, so nothing is
  ;; ever freed.
  (global $heap (mut i32) (i32.const 1024))

  (func $alloc (export "livspeil_alloc") (param $len i32) (result i32)
    (local $ptr i32) (local $end i32) (local $have i32)
    (local.set $ptr
      (i32.and (i32.add (global.get $heap) (i32.const 7)) (i32.const -8)))
    (local.set $end (i32.add (local.get $ptr) (local.get $len)))
    (local.set $have (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $have))
      (then
        (if (i32.eq
              (memory.grow
                (i32.shr_u
                  (i32.add (i32.sub (local.get $end) (local.get $have))
                           (i32.const 65535))
                  (i32.const 16)))
              (i32.const -1))
          (then unreachable))))
    (global.set $heap (local.get $end))
    (local.get $ptr))

  ;; (ptr << 32) | len
  (func $pack (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
      (i64.extend_i32_u (local.get $len))))

  (func (export "livspeil_describe") (result i64)
    (call $pack (i32.const 0) (i32.const 331)))

  ;; sin(2 pi t), deterministic: reduce t to one cycle, fold into
  ;; [-1/4, 1/4] and evaluate the Taylor series to x^13 (error < 1e-9).
  (func $sin_cycles (param $t f64) (result f64)
    (local $u f64) (local $x f64) (local $z f64)
    (local.set $u (f64.sub (local.get $t) (f64.floor (local.get $t))))
    (if (f64.ge (local.get $u) (f64.const 0.75))
      (then (local.set $u (f64.sub (local.get $u) (f64.const 1))))
      (else
        (if (f64.ge (local.get $u) (f64.const 0.25))
          (then (local.set $u (f64.sub (f64.const 0.5) (local.get $u)))))))
    (local.set $x (f64.mul (local.get $u) (f64.const 6.283185307179586)))
    (local.set $z (f64.mul (local.get $x) (local.get $x)))
    (f64.mul (local.get $x)
      (f64.add (f64.const 1)
      (f64.mul (local.get $z)
      (f64.add (f64.const -0.16666666666666666)
      (f64.mul (local.get $z)
      (f64.add (f64.const 0.008333333333333333)
      (f64.mul (local.get $z)
      (f64.add (f64.const -1.984126984126984e-4)
      (f64.mul (local.get $z)
      (f64.add (f64.const 2.755731922398589e-6)
      (f64.mul (local.get $z)
      (f64.add (f64.const -2.505210838544172e-8)
      (f64.mul (local.get $z) (f64.const 1.6059043836821613e-10)))))))))))))))

  ;; Cook input at $in: frame u64 (+0), time f64 (+8), dt f64 (+16),
  ;; input count u32 (+24), then the values (tag u8 + payload). The host
  ;; always passes one value per input port, of the declared type.
  (func (export "livspeil_cook")
      (param $node i32) (param $in i32) (param $len i32) (result i64)
    (local $out i32) (local $n i32) (local $i i32) (local $b i32)
    (block $unknown
      (block $shout
        (block $sine
          (br_table $sine $shout $unknown (local.get $node)))
        ;; sine: values at +28 (frequency), +37 (amplitude), +46 (phase),
        ;; each a Float tag byte then the f64.
        (local.set $out (call $alloc (i32.const 14)))
        (i32.store8 (local.get $out) (i32.const 0))                  ;; ok
        (i32.store offset=1 (local.get $out) (i32.const 1))          ;; 1 value
        (i32.store8 offset=5 (local.get $out) (i32.const 0))         ;; Float
        (f64.store offset=6 (local.get $out)
          (f64.mul
            (f64.load offset=38 (local.get $in))
            (call $sin_cycles
              (f64.add
                (f64.mul (f64.load offset=29 (local.get $in))
                         (f64.load offset=8 (local.get $in)))
                (f64.load offset=47 (local.get $in))))))
        (return (call $pack (local.get $out) (i32.const 14))))
      ;; shout: String tag at +28, length at +29, bytes from +33.
      (local.set $n (i32.load offset=29 (local.get $in)))
      (if (i32.eqz (local.get $n))
        (then (return (call $pack (i32.const 512) (i32.const 27)))))
      ;; ok, 1 value, String tag, length n + 1, bytes, "!"
      (local.set $out (call $alloc (i32.add (local.get $n) (i32.const 11))))
      (i32.store8 (local.get $out) (i32.const 0))
      (i32.store offset=1 (local.get $out) (i32.const 1))
      (i32.store8 offset=5 (local.get $out) (i32.const 7))
      (i32.store offset=6 (local.get $out) (i32.add (local.get $n) (i32.const 1)))
      (block $done
        (loop $next
          (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
          (local.set $b
            (i32.load8_u offset=33 (i32.add (local.get $in) (local.get $i))))
          ;; 'a'..='z' -> 'A'..='Z'; other bytes (UTF-8 too) unchanged
          (if (i32.lt_u (i32.sub (local.get $b) (i32.const 97)) (i32.const 26))
            (then (local.set $b (i32.sub (local.get $b) (i32.const 32)))))
          (i32.store8 offset=10 (i32.add (local.get $out) (local.get $i))
            (local.get $b))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $next)))
      (i32.store8 offset=10 (i32.add (local.get $out) (local.get $n))
        (i32.const 33))                                              ;; '!'
      (return (call $pack (local.get $out)
        (i32.add (local.get $n) (i32.const 11)))))
    (call $pack (i32.const 576) (i32.const 24)))
)
