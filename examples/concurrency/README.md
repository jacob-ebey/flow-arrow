# concurrency

```text
$ flowarrow run main.flow
jobs: 16
total score: 1632
peak score: 272
total weight: 288
peak weight: 33
```

## Why this example matters

This is the smallest example dedicated to concurrency. It avoids timing,
thread identifiers, sleeps, and shared state; those would make the result
depend on scheduler behavior instead of the FlowArrow graph.

1. **Independent jobs.** `range_step(1, 17, 1)` creates sixteen job ids.
   `map score_job` and `map weight_job` apply separate pure nodes to the same
   ids. The backend can lower both regions to worker threads because each
   element has no data dependency on any other element.

2. **Parallel fanout after the maps.** The `scores` sequence feeds score
   reductions, and the `weights` sequence feeds weight reductions. Once those
   sequences exist, the reductions are independent graph branches.

3. **Named aggregate shape.** The four aggregate results are collected into a
   `JobSummary` struct before rendering. `render_summary` uses `field`
   projections to read the named values, which shows how concurrent branches
   can rejoin into an object-shaped value without losing deterministic output.

## Things to inspect

Use the graph command to see the parallel regions:

```text
$ flowarrow graph main.flow
```

Use the native build output to inspect backend lowering. Pure maps over
`score_job` and `weight_job` emit `fa_parallel_for(...)` calls in
`build/<target>/.cache/runtime.c`.

```mermaid
flowchart TD
  subgraph callable_main["program main"]
    main_value__24args_3a_20Args(["$args: Args"])
    main_input_input_a_281_2c_2017_2c_201_29(["input<br/>(1, 17, 1)"])
    main_call_range_step["range_step"]
    main_input_input_a_281_2c_2017_2c_201_29 -- "value" --> main_call_range_step
    main_value__24jobs(["$jobs"])
    main_call_range_step -- "binds" --> main_value__24jobs
    main_map_map_20score_job["map score_job"]
    main_value__24jobs -- "$jobs" --> main_map_map_20score_job
    main_value__24scores(["$scores"])
    main_map_map_20score_job -- "binds" --> main_value__24scores
    main_map_map_20weight_job["map weight_job"]
    main_value__24jobs -- "$jobs" --> main_map_map_20weight_job
    main_value__24weights(["$weights"])
    main_map_map_20weight_job -- "binds" --> main_value__24weights
    main_reduce_reduce_20add_aidentity_3a_200["reduce add<br/>identity: 0"]
    main_value__24scores -- "$scores" --> main_reduce_reduce_20add_aidentity_3a_200
    main_value__24total_score(["$total_score"])
    main_reduce_reduce_20add_aidentity_3a_200 -- "binds" --> main_value__24total_score
    main_reduce_reduce_20max_aidentity_3a_200["reduce max<br/>identity: 0"]
    main_value__24scores -- "$scores" --> main_reduce_reduce_20max_aidentity_3a_200
    main_value__24peak_score(["$peak_score"])
    main_reduce_reduce_20max_aidentity_3a_200 -- "binds" --> main_value__24peak_score
    main_reduce_reduce_20add_aidentity_3a_200_2["reduce add<br/>identity: 0"]
    main_value__24weights -- "$weights" --> main_reduce_reduce_20add_aidentity_3a_200_2
    main_value__24total_weight(["$total_weight"])
    main_reduce_reduce_20add_aidentity_3a_200_2 -- "binds" --> main_value__24total_weight
    main_reduce_reduce_20max_aidentity_3a_200_2["reduce max<br/>identity: 0"]
    main_value__24weights -- "$weights" --> main_reduce_reduce_20max_aidentity_3a_200_2
    main_value__24peak_weight(["$peak_weight"])
    main_reduce_reduce_20max_aidentity_3a_200_2 -- "binds" --> main_value__24peak_weight
    main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea(["input<br/>JobSummary { total_score: $total_score, peak_score: $peak_score, total_weight: $total_weight, peak_weight: $peak_weight }"])
    main_value__24total_score -- "field" --> main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea
    main_value__24peak_score -- "field" --> main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea
    main_value__24total_weight -- "field" --> main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea
    main_value__24peak_weight -- "field" --> main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea
    main_value__24summary(["$summary"])
    main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea -- "binds" --> main_value__24summary
    main_call_render_summary["render_summary"]
    main_value__24summary -- "$summary" --> main_call_render_summary
    main_value__24output(["$output"])
    main_call_render_summary -- "binds" --> main_value__24output
    main_call_write_stdout[["write_stdout"]]
    main_value__24output -- "$output" --> main_call_write_stdout
    main_value__24exit_code(["$exit_code"])
    main_call_write_stdout -- "binds" --> main_value__24exit_code
  end
  subgraph callable_score_job["node score_job"]
    score_job_value__24n_3a_20Int(["$n: i64"])
    score_job_input_input_a_28_24n_2c_20_24n_29(["input<br/>($n, $n)"])
    score_job_value__24n_3a_20Int -- "item" --> score_job_input_input_a_28_24n_2c_20_24n_29
    score_job_value__24n_3a_20Int -- "item" --> score_job_input_input_a_28_24n_2c_20_24n_29
    score_job_call_mul["mul"]
    score_job_input_input_a_28_24n_2c_20_24n_29 -- "value" --> score_job_call_mul
    score_job_value__24square(["$square"])
    score_job_call_mul -- "binds" --> score_job_value__24square
    score_job_input_input_a_28_24square_2c_20_24n_29(["input<br/>($square, $n)"])
    score_job_value__24square -- "item" --> score_job_input_input_a_28_24square_2c_20_24n_29
    score_job_value__24n_3a_20Int -- "item" --> score_job_input_input_a_28_24square_2c_20_24n_29
    score_job_call_add["add"]
    score_job_input_input_a_28_24square_2c_20_24n_29 -- "value" --> score_job_call_add
    score_job_value__24score(["$score"])
    score_job_call_add -- "binds" --> score_job_value__24score
  end
  subgraph callable_weight_job["node weight_job"]
    weight_job_value__24n_3a_20Int(["$n: i64"])
    weight_job_input_input_a_28_24n_2c_202_29(["input<br/>($n, 2)"])
    weight_job_value__24n_3a_20Int -- "item" --> weight_job_input_input_a_28_24n_2c_202_29
    weight_job_call_mul["mul"]
    weight_job_input_input_a_28_24n_2c_202_29 -- "value" --> weight_job_call_mul
    weight_job_value__24doubled(["$doubled"])
    weight_job_call_mul -- "binds" --> weight_job_value__24doubled
    weight_job_input_input_a_28_24doubled_2c_201_29(["input<br/>($doubled, 1)"])
    weight_job_value__24doubled -- "item" --> weight_job_input_input_a_28_24doubled_2c_201_29
    weight_job_call_add["add"]
    weight_job_input_input_a_28_24doubled_2c_201_29 -- "value" --> weight_job_call_add
    weight_job_value__24weight(["$weight"])
    weight_job_call_add -- "binds" --> weight_job_value__24weight
  end
  subgraph callable_render_summary["node render_summary"]
    render_summary_value__24summary_3a_20JobSummary(["$summary: JobSummary"])
    render_summary_field_field_20total_score["field total_score"]
    render_summary_value__24summary_3a_20JobSummary -- "$summary" --> render_summary_field_field_20total_score
    render_summary_value__24total_score(["$total_score"])
    render_summary_field_field_20total_score -- "binds" --> render_summary_value__24total_score
    render_summary_field_field_20peak_score["field peak_score"]
    render_summary_value__24summary_3a_20JobSummary -- "$summary" --> render_summary_field_field_20peak_score
    render_summary_value__24peak_score(["$peak_score"])
    render_summary_field_field_20peak_score -- "binds" --> render_summary_value__24peak_score
    render_summary_field_field_20total_weight["field total_weight"]
    render_summary_value__24summary_3a_20JobSummary -- "$summary" --> render_summary_field_field_20total_weight
    render_summary_value__24total_weight(["$total_weight"])
    render_summary_field_field_20total_weight -- "binds" --> render_summary_value__24total_weight
    render_summary_field_field_20peak_weight["field peak_weight"]
    render_summary_value__24summary_3a_20JobSummary -- "$summary" --> render_summary_field_field_20peak_weight
    render_summary_value__24peak_weight(["$peak_weight"])
    render_summary_field_field_20peak_weight -- "binds" --> render_summary_value__24peak_weight
    render_summary_call_format_int["format_int"]
    render_summary_value__24total_score -- "$total_score" --> render_summary_call_format_int
    render_summary_value__24total_score_bytes(["$total_score_bytes"])
    render_summary_call_format_int -- "binds" --> render_summary_value__24total_score_bytes
    render_summary_call_format_int_2["format_int"]
    render_summary_value__24peak_score -- "$peak_score" --> render_summary_call_format_int_2
    render_summary_value__24peak_score_bytes(["$peak_score_bytes"])
    render_summary_call_format_int_2 -- "binds" --> render_summary_value__24peak_score_bytes
    render_summary_call_format_int_3["format_int"]
    render_summary_value__24total_weight -- "$total_weight" --> render_summary_call_format_int_3
    render_summary_value__24total_weight_bytes(["$total_weight_bytes"])
    render_summary_call_format_int_3 -- "binds" --> render_summary_value__24total_weight_bytes
    render_summary_call_format_int_4["format_int"]
    render_summary_value__24peak_weight -- "$peak_weight" --> render_summary_call_format_int_4
    render_summary_value__24peak_weight_bytes(["$peak_weight_bytes"])
    render_summary_call_format_int_4 -- "binds" --> render_summary_value__24peak_weight_bytes
    render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot(["input<br/>[&quot;jobs: 16\n&quot;, &quot;total score: &quot;, $total_score_bytes, &quot;\n&quot;, &quot;peak score: &quot;, $peak_score_bytes, &quot;\n&quot;, &quot;total weight: &quot;, $total_weight_bytes, &quot;\n&quot;, &quot;peak weight: &quot;, $peak_weight_bytes, &quot;\n&quot;]"])
    render_summary_value__24total_score_bytes -- "item" --> render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot
    render_summary_value__24peak_score_bytes -- "item" --> render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot
    render_summary_value__24total_weight_bytes -- "item" --> render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot
    render_summary_value__24peak_weight_bytes -- "item" --> render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot
    render_summary_call_concat_bytes["concat_bytes"]
    render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot -- "value" --> render_summary_call_concat_bytes
    render_summary_value__24output(["$output"])
    render_summary_call_concat_bytes -- "binds" --> render_summary_value__24output
  end
  subgraph legend["legend"]
    legend_value_value_20_2f_20binding(["value / binding"])
    legend_op_pure_20operation["pure operation"]
    legend_boundary_boundary_20operation[["boundary operation"]]
    legend_collection_collection_20operator["collection operator"]
    legend_decision_match_20_2f_20decision{"match / decision"}
    legend_fault_fault_20path["fault path"]
  end
  classDef value fill:#e8f4ff,stroke:#2f6f9f,color:#102a43
  classDef literal fill:#f7f9fb,stroke:#9aa6b2,color:#1f2933
  classDef op fill:#ffffff,stroke:#59636e,color:#111827
  classDef boundary fill:#fff4df,stroke:#b87918,color:#3f2a05
  classDef collection fill:#ecfdf3,stroke:#2f855a,color:#123524
  classDef decision fill:#f4ecff,stroke:#7c3aed,color:#2d124d
  classDef fault fill:#ffecec,stroke:#c64242,color:#5a1111
  class main_value__24args_3a_20Args value
  class main_input_input_a_281_2c_2017_2c_201_29 literal
  class main_call_range_step op
  class main_value__24jobs value
  class main_map_map_20score_job collection
  class main_value__24scores value
  class main_map_map_20weight_job collection
  class main_value__24weights value
  class main_reduce_reduce_20add_aidentity_3a_200 collection
  class main_value__24total_score value
  class main_reduce_reduce_20max_aidentity_3a_200 collection
  class main_value__24peak_score value
  class main_reduce_reduce_20add_aidentity_3a_200_2 collection
  class main_value__24total_weight value
  class main_reduce_reduce_20max_aidentity_3a_200_2 collection
  class main_value__24peak_weight value
  class main_input_input_aJobSummary_20_7b_20total_score_3a_20_24total_score_2c_20peak_score_3a_20_24pea literal
  class main_value__24summary value
  class main_call_render_summary op
  class main_value__24output value
  class main_call_write_stdout boundary
  class main_value__24exit_code value
  class score_job_value__24n_3a_20Int value
  class score_job_input_input_a_28_24n_2c_20_24n_29 literal
  class score_job_call_mul op
  class score_job_value__24square value
  class score_job_input_input_a_28_24square_2c_20_24n_29 literal
  class score_job_call_add op
  class score_job_value__24score value
  class weight_job_value__24n_3a_20Int value
  class weight_job_input_input_a_28_24n_2c_202_29 literal
  class weight_job_call_mul op
  class weight_job_value__24doubled value
  class weight_job_input_input_a_28_24doubled_2c_201_29 literal
  class weight_job_call_add op
  class weight_job_value__24weight value
  class render_summary_value__24summary_3a_20JobSummary value
  class render_summary_field_field_20total_score op
  class render_summary_value__24total_score value
  class render_summary_field_field_20peak_score op
  class render_summary_value__24peak_score value
  class render_summary_field_field_20total_weight op
  class render_summary_value__24total_weight value
  class render_summary_field_field_20peak_weight op
  class render_summary_value__24peak_weight value
  class render_summary_call_format_int op
  class render_summary_value__24total_score_bytes value
  class render_summary_call_format_int_2 op
  class render_summary_value__24peak_score_bytes value
  class render_summary_call_format_int_3 op
  class render_summary_value__24total_weight_bytes value
  class render_summary_call_format_int_4 op
  class render_summary_value__24peak_weight_bytes value
  class render_summary_input_input_a_5b_22jobs_3a_2016_5cn_22_2c_20_22total_20score_3a_20_22_2c_20_24tot literal
  class render_summary_call_concat_bytes op
  class render_summary_value__24output value
  class legend_value_value_20_2f_20binding value
  class legend_op_pure_20operation op
  class legend_boundary_boundary_20operation boundary
  class legend_collection_collection_20operator collection
  class legend_decision_match_20_2f_20decision decision
  class legend_fault_fault_20path fault
```
