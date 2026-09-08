# Support enquiry — rebel-compiler 0.10.3 fails on a graph that 0.10.2 compiled

**Ready to send.** Facts only; every claim below is backed by a log kept in this repo or on
the node. The KONI-8B control run under 0.10.3 completed on 2026-09-08 and failed as
predicted, so the reproduction is a single-variable one: only the toolchain version differs
from the build that succeeded.

---

**Subject:** rebel-compiler 0.10.3 — "Error occurred while compiling the model" on a model
that 0.10.2 compiled successfully on the same host

Hello,

We compile models for RBLN-CA22 as part of an llm-d deployment. A model that compiled
successfully on this host under RBLN SDK **0.10.2** now fails under **0.10.3**, with the
generic wrapper message and no further detail. We would like either guidance on the cause or
access to a 0.10.2 bundle so we can proceed.

## What worked (2026-06-01, SDK 0.10.2)

```python
model = RBLNAutoModelForCausalLM.from_pretrained(
    model_id="KISTI-KONI/KONI-Llama3.1-8B-Instruct-20241024", export=True,
    rbln_batch_size=1, rbln_max_seq_len=8192, rbln_tensor_parallel_size=4,
    rbln_create_runtimes=False, rbln_npu="RBLN-CA22")
```

```
INFO [rebel-compiler] RBLN SDK compiler version: 0.10.2
INFO [rebel-compiler] -- Target NPU: RBLN-CA22
WARNING [rebel-compiler] Could not determine the current machine's NPU. Skipping NPU mismatch check.
INFO [rebel-compiler] -- Tensor parallel size: 4
INFO [rebel-compiler] |Compile(#0), mod_name=1a4e47, input_info_index=0|
  → completed; artifact prefill.rbln 17.2GB + decoder_batch_1.rbln 2.2GB
  → rbln_config.json records "optimum_rbln_version": "0.10.2"
```

That artifact is still in production service on this cluster.

## What fails (SDK 0.10.3)

Same model, same parameters, same host. Only the toolchain differs:
`optimum-rbln 0.10.3, rebel-compiler 0.10.3, transformers 4.57.6, torch 2.10.0+cpu`
(installed from the matched vendor wheel bundle, `pip install --no-index`).

```
INFO [rebel-compiler] RBLN SDK compiler version: 0.10.3
INFO [rebel-compiler] -- Tensor parallel size: 4
INFO [rebel-compiler] |Compile(#0), mod_name=1a4e47, input_info_index=0|
Computation graph generation   ... 100%
Computation graph optimization ... 100%
  File "/usr/local/.../rebel/compile_from_any.py", line 231, in _compile_from_torch
    compiled_model = compile(mod, compile_context=compile_context, npu=npu, **kwargs)
  File "<frozen core.compilation._impl>", line 946, in compile
RuntimeError: Error occurred while compiling the model
```

**`mod_name=1a4e47` is identical to the 0.10.2 log above** — the graph handed to the compiler
is the same one that compiled successfully. Both progress phases reach 100%; the failure is
after graph optimization, during result assembly.

## What we ruled out by experiment

| Hypothesis | How it was tested | Result |
|---|---|---|
| Compile parameters | 5-rung ladder ending at the exact 0.10.2 parameters | not the cause |
| Model / architecture | Qwen2.5-0.5B, Llama-3.2-1B, Llama-3.1-8B, **KONI-8B (the exact model 0.10.2 compiled)** | all fail alike |
| Dependency versions | vendor-pinned bundle, verified against `importlib.metadata.requires` | not the cause |
| NPU visibility | 0.10.2 succeeded while warning it could not determine the NPU | not the cause |
| Node environment drift | reproduced in a clean `python:3.10-slim` container | not the cause |
| Host toolchain 0.11.0.post1 | same failure, at `_impl:974` | not version-specific to 0.10.3 |
| `RBLN_COMPILER_LOG_LEVEL`, `RBLN_DEBUG_LEVEL` | both refused: "dev-only and cannot be used in a deploy build" | no diagnostics available |

The Python exception carries no `__cause__`; the message originates in native code in
`librbln.so`.

## What we are asking for

1. **A 0.10.2 wheel bundle**, so we can restore the configuration that demonstrably works
   here. We already hold a matched 0.10.3 bundle, so the same delivery route is fine.
2. Failing that, **a development build** (or a documented way to raise compiler log level in a
   deploy build) so the underlying error becomes visible.
3. Any known regression between 0.10.2 and 0.10.3 affecting Llama-family decoder-only models
   at `rbln_tensor_parallel_size=4`.

Environment: Ubuntu, Python 3.10, RBLN-CA22 × 4 on the compile host; compilation performed
without claiming devices (`rbln_create_runtimes=False`).

Thank you.

---

## Control run — closed

The KONI-8B control under 0.10.3 (the exact model and parameters of the 2026-06-01 success)
completed 2026-09-08 10:17 and failed at `_impl:946`, container exit 0, no OOM, graph hash
`mod_name=1a4e47` matching the successful June log. Two earlier attempts proved nothing and
are recorded so they are not mistaken for evidence: the first was OOMKilled at a 64Gi
container limit, the second ran the wrong script because a `sed` was chained off an
unmodified bootstrap file.
