# Runtime candidate measurements — 2026-07-12

## Predeclared protocol and materiality thresholds

These thresholds were written before collecting performance runs. A result opens a separately reviewed optimization plan only when all nine runs satisfy the correctness gate and the candidate shows material harm:

- Startup scan: `output_equal=true` and maximum scheduling delay exceeds 100 ms in at least 5/9 runs.
- Logger fan-out: `output_equal=true`; open a plan to enforce one active viewer only if slow readers cause either healthy-viewer latency above 500 ms or file-log latency above 500 ms in at least 5/9 runs, or any dropped messages in at least 5/9 runs.
- Stream flush: exact output equality in both variants; no-per-read-flush must keep first-byte latency at or below 25 ms and maximum inter-chunk latency at or below 100 ms in every run, and improve median throughput by at least 10% to open a plan.

Fixture sizes are encoded in every NDJSON record. Latency thresholds are responsiveness limits, not benchmark timeouts. Run on an otherwise idle machine where feasible. Preserve every record, including failures and outliers.

Implementation caveat: the production startup scan and stream pump are crate-private, and the production logger is a process-global singleton. To preserve the no-production-change boundary, this harness measures benchmark-local replicas of their relevant synchronous scan, sequential fan-out/file-write, and per-read-flush I/O behavior. It does not claim end-to-end equivalence.

## Environment

- Git SHA: `6fba4546a4d1a3571185115de0c776d208363f1e`
- OS: Darwin 24.6.0 arm64
- Toolchain: `rustc 1.96.1 (31fca3adb 2026-06-26)`, `cargo 1.96.1 (356927216 2026-06-26)`
- Condition: serial runs on an otherwise idle machine; normal OS background services remained active.

## Commands and raw results

### `cargo bench --bench runtime_candidates -- startup-scan --runs 9`

```ndjson
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.186875,"output_equal":true,"run":1,"scan_ms":11.019875}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.162042,"output_equal":true,"run":2,"scan_ms":7.387625}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.176125,"output_equal":true,"run":3,"scan_ms":8.449417}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.15475,"output_equal":true,"run":4,"scan_ms":7.278166}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.26033300000000004,"output_equal":true,"run":5,"scan_ms":7.551292}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.20066699999999998,"output_equal":true,"run":6,"scan_ms":7.773458}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.18725,"output_equal":true,"run":7,"scan_ms":8.661541999999999}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":2.9779590000000002,"output_equal":true,"run":8,"scan_ms":9.313666999999999}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":0.180375,"output_equal":true,"run":9,"scan_ms":10.090667}
```

### `cargo bench --bench runtime_candidates -- logger-fanout --runs 9`

```ndjson
{"case":"logger-fanout","dropped_messages":60,"file_log_latency_ms":6146.545625,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5735.722,"output_equal":true,"run":1,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":61,"file_log_latency_ms":6241.924333,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5833.726708,"output_equal":true,"run":2,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":60,"file_log_latency_ms":6141.2885,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5732.020208,"output_equal":true,"run":3,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":60,"file_log_latency_ms":6140.755792,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5731.628792,"output_equal":true,"run":4,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":60,"file_log_latency_ms":6140.443708000001,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5730.427709,"output_equal":true,"run":5,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":61,"file_log_latency_ms":6256.48975,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5848.616040999999,"output_equal":true,"run":6,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":60,"file_log_latency_ms":6139.001791,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5731.879207999999,"output_equal":true,"run":7,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":59,"file_log_latency_ms":6039.644084,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5630.5221249999995,"output_equal":true,"run":8,"slow_clients":4}
{"case":"logger-fanout","dropped_messages":59,"file_log_latency_ms":6039.074584,"fixture_message_bytes":1048576,"fixture_messages":16,"healthy_viewer_latency_ms":5630.533584,"output_equal":true,"run":9,"slow_clients":4}
```

### `cargo bench --bench runtime_candidates -- stream-flush --runs 9`

```ndjson
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.02325,"max_inter_chunk_latency_ms":0.08433399999999999,"throughput_mib_s":4905.4655470759135},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.15512499999999999,"max_inter_chunk_latency_ms":1.8465829999999999,"throughput_mib_s":1095.5902492467817},"run":1}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.022542,"max_inter_chunk_latency_ms":0.022542,"throughput_mib_s":5819.586983911752},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.023167,"max_inter_chunk_latency_ms":0.0885,"throughput_mib_s":4227.771171885322},"run":2}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.010875,"max_inter_chunk_latency_ms":0.030957999999999996,"throughput_mib_s":3928.790669122161},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.025041,"max_inter_chunk_latency_ms":0.025041,"throughput_mib_s":5478.829119428887},"run":3}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.022708,"max_inter_chunk_latency_ms":0.046084,"throughput_mib_s":5174.088681292953},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.022917,"max_inter_chunk_latency_ms":0.07629100000000001,"throughput_mib_s":4482.837456796654},"run":4}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.017750000000000002,"max_inter_chunk_latency_ms":0.140334,"throughput_mib_s":4248.350047050477},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.031166,"max_inter_chunk_latency_ms":0.17133299999999999,"throughput_mib_s":4109.94091959928},"run":5}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.019916,"max_inter_chunk_latency_ms":0.063417,"throughput_mib_s":4753.891059832473},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.019792,"max_inter_chunk_latency_ms":0.082958,"throughput_mib_s":3536.6931918656055},"run":6}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.019042000000000003,"max_inter_chunk_latency_ms":0.13454100000000002,"throughput_mib_s":4296.261607961832},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.021709,"max_inter_chunk_latency_ms":0.2415,"throughput_mib_s":4414.601736704323},"run":7}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.016958,"max_inter_chunk_latency_ms":0.36666699999999997,"throughput_mib_s":2722.3996319315697},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.022,"max_inter_chunk_latency_ms":0.136167,"throughput_mib_s":3713.8755963091503},"run":8}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.014,"max_inter_chunk_latency_ms":0.029667,"throughput_mib_s":6171.249076241154},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.024292,"max_inter_chunk_latency_ms":0.1,"throughput_mib_s":5444.339792135106},"run":9}
```

## Decisions

- Startup scheduling: **retain current behavior**. Zero of nine runs exceeded 100 ms; the preserved maximum outlier was 2.978 ms.
- Logger fan-out: **open a separately reviewed optimization plan** limited to documenting/enforcing one active viewer. All nine runs exceeded both 500 ms latency limits and reported timed-out client writes. Do not add per-client tasks or queues based on this benchmark.
- Stream flushing: **open a separately reviewed optimization plan**. Exact output and both responsiveness limits passed in all runs. Median throughput improved from 4227.77 MiB/s to 4753.89 MiB/s (12.4%), exceeding the predeclared 10% threshold. Do not modify `pump_raw_output` without that separate review.
