# Runtime candidate measurements — 2026-07-12

Git `6fba4546a4d1a3571185115de0c776d208363f1e`; Darwin 24.6.0 arm64; rustc/cargo 1.96.1. Serial runs on an otherwise idle machine, excluding normal OS services.

Predeclared thresholds: startup material at >100 ms in 5/9; logger material at >500 ms file/viewer latency or any channel drops in 5/9; stream requires exact output, <=25 ms first byte, <=100 ms gap, and >=10% median throughput improvement. Because the stream sink is not terminal-representative, stream evidence is inconclusive regardless of threshold. All earlier invalid measurements were discarded after review and are not decisions.

Commands: `cargo bench --bench runtime_candidates -- startup-scan --runs 9`; same for `logger-fanout` and `stream-flush`.

## Complete raw decision records

```ndjson
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":19.408125,"output_equal":true,"run":1,"scan_ms":20.379167}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":14.520417,"output_equal":true,"run":2,"scan_ms":15.497125}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":15.612167,"output_equal":true,"run":3,"scan_ms":16.586333000000003}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":13.669542,"output_equal":true,"run":4,"scan_ms":14.646958}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":14.731458,"output_equal":true,"run":5,"scan_ms":15.707417000000001}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":15.415083,"output_equal":true,"run":6,"scan_ms":16.383167}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":14.858959,"output_equal":true,"run":7,"scan_ms":15.835875}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":16.882458,"output_equal":true,"run":8,"scan_ms":17.858124999999998}
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":14.726875,"output_equal":true,"run":9,"scan_ms":15.701083}
{"case":"logger-fanout","dropped_messages":375,"file_log_latency_ms":414.460084,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":420.574333,"output_equal":true,"run":1,"slow_clients":4,"total_elapsed_ms":420.54449999999997}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":437.173666,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":441.1215,"output_equal":true,"run":2,"slow_clients":4,"total_elapsed_ms":441.08133300000003}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":415.29120800000004,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":421.699,"output_equal":true,"run":3,"slow_clients":4,"total_elapsed_ms":421.62179199999997}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":411.886792,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":416.05966700000005,"output_equal":true,"run":4,"slow_clients":4,"total_elapsed_ms":416.030833}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":415.566417,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":420.301125,"output_equal":true,"run":5,"slow_clients":4,"total_elapsed_ms":420.270834}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":412.744375,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":419.614917,"output_equal":true,"run":6,"slow_clients":4,"total_elapsed_ms":419.560833}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":412.045416,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":416.561083,"output_equal":true,"run":7,"slow_clients":4,"total_elapsed_ms":416.512459}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":413.911292,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":423.17808299999996,"output_equal":true,"run":8,"slow_clients":4,"total_elapsed_ms":423.15333300000003}
{"case":"logger-fanout","dropped_messages":395,"file_log_latency_ms":415.271875,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":420.19154199999997,"output_equal":true,"run":9,"slow_clients":4,"total_elapsed_ms":420.11758399999997}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.05375,"max_inter_chunk_latency_ms":0.05375,"throughput_mib_s":2335.1977941721634},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.054084,"max_inter_chunk_latency_ms":0.054084,"throughput_mib_s":2380.4210369709144},"run":1}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.050791,"max_inter_chunk_latency_ms":0.050791,"throughput_mib_s":2518.5618004694597},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.067458,"max_inter_chunk_latency_ms":0.067458,"throughput_mib_s":2604.025954326687},"run":2}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.027833,"max_inter_chunk_latency_ms":0.027833,"throughput_mib_s":2670.9704904502787},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.047125,"max_inter_chunk_latency_ms":0.047125,"throughput_mib_s":2633.890532875563},"run":3}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.038167,"max_inter_chunk_latency_ms":0.038167,"throughput_mib_s":2742.073693230506},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.027041000000000003,"max_inter_chunk_latency_ms":0.027041000000000003,"throughput_mib_s":2779.949613413257},"run":4}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.024833,"max_inter_chunk_latency_ms":0.024833,"throughput_mib_s":2778.0189252539285},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.038209,"max_inter_chunk_latency_ms":0.038209,"throughput_mib_s":2711.097947224412},"run":5}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.046375,"max_inter_chunk_latency_ms":0.046375,"throughput_mib_s":2701.7151162987043},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.028958,"max_inter_chunk_latency_ms":0.028958,"throughput_mib_s":2790.050123250464},"run":6}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.027208,"max_inter_chunk_latency_ms":0.027208,"throughput_mib_s":2790.616551844423},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.039834,"max_inter_chunk_latency_ms":0.039834,"throughput_mib_s":2415.1541260794984},"run":7}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.036833,"max_inter_chunk_latency_ms":0.036833,"throughput_mib_s":2712.4786222778584},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.029249999999999998,"max_inter_chunk_latency_ms":0.029249999999999998,"throughput_mib_s":2725.027676062335},"run":8}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.062625,"max_inter_chunk_latency_ms":0.062625,"throughput_mib_s":2682.3890429772373},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.041875,"max_inter_chunk_latency_ms":0.041875,"throughput_mib_s":2481.454231437792},"run":9}
```

## Decisions and failures/outliers

- Startup: retain. The actual `src/monitor/startup.rs` is compiled benchmark-only with adapters; all delays were 13.670–19.408 ms, below threshold. Run 1 is the maximum preserved outlier.
- Logger: retain. Timestamped markers were enqueued while slow clients were connected; dead clients were removed and actual channel-full drops counted. Drops occurred 9/9, but without a no-slow-client control attribution remains inconclusive.
- Stream: retain; terminal causality remains inconclusive. Run 7 per-read throughput is the preserved low outlier.
- No corrected decision run failed. Earlier review-invalid runs were superseded, not silently summarized into these decisions.

## Verification and smoke records

`cargo fmt --check`: pass. `cargo test`: pass (57 tests). `cargo clippy --all-targets --all-features -- -D warnings`: pass.

```ndjson
{"case":"startup-scan","fixture_bytes":67108864,"fixture_files":256,"max_scheduling_delay_ms":12.084332999999999,"output_equal":true,"run":1,"scan_ms":13.065209}
{"case":"logger-fanout","dropped_messages":387,"file_log_latency_ms":413.505708,"fixture_message_bytes":32768,"fixture_messages":512,"healthy_viewer_latency_ms":416.26625,"output_equal":true,"run":1,"slow_clients":4,"total_elapsed_ms":416.17525}
{"case":"stream-flush","fixture_bytes":4194304,"fixture_chunks":512,"no_per_read_flush":{"first_byte_latency_ms":0.052958,"max_inter_chunk_latency_ms":0.052958,"throughput_mib_s":2209.9960330571207},"output_equal":true,"per_read_flush":{"first_byte_latency_ms":0.058083,"max_inter_chunk_latency_ms":0.07275000000000001,"throughput_mib_s":2252.5693368999014},"run":1}
```
