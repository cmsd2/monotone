# Scenario coverage for refresh-2026

Every scenario in the post-change specs (baseline specs with this change's deltas
applied) maps to at least one automated test or CI mechanism.

Test locations:

- **lib**: unit tests in `monotone/src` (`cargo test -p monotone`)
- **it**: `monotone/tests/dynamodb.rs`, against DynamoDB Local
- **wire**: `monotone/tests/dynamodb_wire.rs`, scripted HTTP client, no network
- **cli**: `cli/tests/cli.rs`, the built binary, against DynamoDB Local
- **ci**: `.github/workflows/*.yml`

## sans-io-core

| Scenario | Verified by |
|---|---|
| Read-only operation | lib `core::ops::tests::read_only_emits_one_read_then_completes` |
| Read-only operation on a missing row | lib `core::ops::tests::get_tickets_on_missing_row_is_empty_without_write` |
| Mutating operation without contention | lib `core::ops::tests::next_value_without_contention` |
| Input does not match the pending effect | lib `core::ops::tests::mismatched_input_is_a_protocol_error_and_finishes`, `rmw_rejects_out_of_order_calls` |
| Three conflicts then success | lib `core::ops::tests::three_conflicts_then_success` |
| Retry observes a rejoin | lib `core::ops::tests::retry_observes_a_rejoin`, `core::props::rejoin_after_interleaved_join_returns_the_winners_ticket` |
| Retry observes a removal | lib `core::ops::tests::retry_observes_a_removal` |
| Same seed, same sleeps | lib `core::ops::tests::same_seed_gives_same_sleeps_within_bounds` |
| Counter value is never reused | lib `core::row::tests::counter_is_not_reused_after_leave`, `core::props::invariants_hold_under_interleaved_conflicts` |
| Positions are dense after leave | lib `core::row::tests::positions_are_dense_after_leave_and_counters_unchanged`, `core::props::invariants_hold_under_interleaved_conflicts` |
| Version bumps exactly once per write | lib `core::row::tests::version_bumps_exactly_once_per_write`, `core::props::invariants_hold_under_interleaved_conflicts` |
| Round trip | lib `core::props::counter_row_round_trips`, `core::props::queue_row_round_trips` |
| Items are sorted on decode | lib `core::encode::tests::items_are_sorted_by_counter_on_decode` |
| Same script, same results | it `memory_and_dynamodb_backends_agree` |
| Default build | ci `lint` step "Default library build has no I/O dependencies" |

## counter

| Scenario | Verified by |
|---|---|
| Read a fresh counter | lib `memory::tests::fresh_counter_reads_zero`; it `counter_read_without_item_is_zero_and_creates_nothing` |
| Increment returns the new value | lib `memory::tests::increment_returns_new_value_and_get_agrees`; it `counter_repeated_increments_and_remove` |
| Repeated increments | lib `memory::tests::increment_returns_new_value_and_get_agrees`; it `counter_repeated_increments_and_remove` |
| Concurrent increments in one process | lib `memory::tests::concurrent_increments_and_joins_never_collide`, `counter_clones_and_handles_share_state` |
| Read when no item exists | it `counter_read_without_item_is_zero_and_creates_nothing` |
| First increment creates the item | it `counter_first_increment_creates_item` |
| Increment retries on a concurrent write | it `concurrent_next_value_on_two_clients_yields_consecutive_values`; lib `dynamodb::adapter::tests::conflict_sleeps_then_rereads_and_writes_on_top` |
| Item is not a counter | it `counter_on_queue_item_and_queue_on_counter_item_are_wrong_type`; lib `memory::tests::counter_on_queue_row_is_wrong_type_and_vice_versa` |
| Item is missing required attributes | it `counter_with_missing_or_bad_attributes_names_them`; lib `core::encode::tests::each_missing_required_attribute_is_named`, `non_integer_numbers_are_rejected` |
| Remove then read | it `counter_repeated_increments_and_remove`; lib `memory::tests::counter_remove_resets_and_tolerates_missing` |
| Remove a missing counter | it `counter_repeated_increments_and_remove`; lib `memory::tests::counter_remove_resets_and_tolerates_missing` |

## queue

| Scenario | Verified by |
|---|---|
| Ticket shape | lib `memory::tests::join_ticket_shape_and_numbering` |
| Empty queue token | lib `memory::tests::empty_queue_lists_token_zero`; it `queue_numbering_tokens_and_item_encoding` |
| Token increments per write | lib `memory::tests::token_increments_once_per_write`; it `queue_numbering_tokens_and_item_encoding` |
| Read returns the current token | lib `memory::tests::read_returns_current_token_and_same_ticket`; it `queue_numbering_tokens_and_item_encoding` |
| First join | lib `memory::tests::join_ticket_shape_and_numbering`; it `queue_numbering_tokens_and_item_encoding` |
| Second join | lib `memory::tests::join_ticket_shape_and_numbering`; it `queue_numbering_tokens_and_item_encoding` |
| Join with tags | lib `memory::tests::join_with_tags_round_trips_through_every_read`; it `queue_rejoin_tags_and_lookups` |
| Rejoin | lib `memory::tests::rejoin_is_idempotent`; it `queue_rejoin_tags_and_lookups` |
| Leave shifts later positions | lib `memory::tests::leave_shifts_later_positions_and_keeps_counters`; it `queue_numbering_tokens_and_item_encoding` |
| Leave a process that is not queued | lib `memory::tests::leave_unknown_process_is_not_found_and_token_unchanged`; it `queue_rejoin_tags_and_lookups` |
| Lookup of an unknown process | lib `memory::tests::leave_and_lookup_on_missing_queue_are_not_found`; it `queue_rejoin_tags_and_lookups` |
| List after joins | lib `memory::tests::list_after_joins_is_in_position_order`; it `concurrent_joins_from_many_clients_get_distinct_counters` |
| First counter value in memory | lib `memory::tests::join_ticket_shape_and_numbering` |
| Concurrent joins in one process | lib `memory::tests::concurrent_increments_and_joins_never_collide` |
| First counter value on DynamoDB | it `queue_numbering_tokens_and_item_encoding` |
| Read when no item exists | it `queue_numbering_tokens_and_item_encoding` |
| Join retries on a concurrent write | it `concurrent_joins_from_many_clients_get_distinct_counters`; lib `core::ops::tests::retry_observes_a_rejoin` |
| Leave on a missing queue returns promptly | it `leave_and_lookup_on_missing_queue_return_promptly`; lib `core::ops::tests::leave_on_missing_row_fails_immediately` |
| Item is not a queue | it `counter_on_queue_item_and_queue_on_counter_item_are_wrong_type` |
| Entries are ordered by counter on read | lib `core::encode::tests::items_are_sorted_by_counter_on_decode`; it `reads_and_extends_a_0_4_queue_item` |
| Remove then list | lib `memory::tests::queue_remove_resets_everything`; it `queue_remove_resets_token_and_counter` |
| First and second joiner | lib `core::row::tests::first_and_second_joiner_get_counters_one_and_two`; it `memory_and_dynamodb_backends_agree` |

## dynamodb-storage

| Scenario | Verified by |
|---|---|
| Table created by the library | it `table_create_describe_and_errors`; wire `create_table_if_needed_creates_a_missing_table` |
| Counter item | lib `core::encode::tests::counter_item_fixture`; it `counter_first_increment_creates_item` |
| Queue item | lib `core::encode::tests::queue_item_fixture`; it `queue_numbering_tokens_and_item_encoding` |
| Empty queue after leave | lib `core::encode::tests::empty_queue_omits_items`; it `queue_numbering_tokens_and_item_encoding` |
| Missing Items on read | lib `core::encode::tests::missing_items_decodes_as_empty` |
| Counter command on a queue row | it `counter_on_queue_item_and_queue_on_counter_item_are_wrong_type`; lib `core::encode::tests::wrong_type_is_rejected_both_ways` |
| Read after write | wire `get_item_is_strongly_consistent`; it `counter_repeated_increments_and_remove` |
| Write against an unchanged item | it `counter_repeated_increments_and_remove`; wire `put_item_carries_condition_on_the_wire` |
| Write against a changed item | it `two_clients_racing_a_conditional_put_exactly_one_wins` |
| Write to create a new item | it `counter_first_increment_creates_item`; lib `core::ops::tests::first_write_to_missing_row_expects_version_zero` |
| Two writers race to create | it `two_clients_racing_a_conditional_put_exactly_one_wins` |
| Retry re-reads state | lib `dynamodb::adapter::tests::conflict_sleeps_then_rereads_and_writes_on_top`, `memory::tests::second_writer_conflicts_rereads_and_reflects_the_winner` |
| Non-conditional error | wire `non_conditional_put_failure_aborts_without_retry`; lib `dynamodb::adapter::tests::executor_errors_abort_without_retry` |
| Table absent | wire `create_table_if_needed_creates_a_missing_table`; it `table_create_describe_and_errors` |
| Table present | wire `create_table_if_needed_leaves_an_existing_table_alone`; it `table_create_describe_and_errors` |
| Concurrent creation | wire `create_table_if_needed_tolerates_a_concurrent_creator`; it `concurrent_table_creation_is_tolerated` |
| Waiting for activation | wire `wait_for_table_polls_until_active` |
| Conditional check failure | wire `service_errors_are_classified_by_type`; it `two_clients_racing_a_conditional_put_exactly_one_wins` |
| Table missing | wire `service_errors_are_classified_by_type`; it `table_create_describe_and_errors` |
| Unknown error passes through | wire `service_errors_are_classified_by_type`; it `missing_table_and_unreachable_endpoint_are_wrapped_sdk_errors` |
| Delete | it `counter_repeated_increments_and_remove`, `queue_remove_resets_token_and_counter` |
| DynamoDB Local | it (whole suite runs via `AWS_ENDPOINT_URL`); wire `endpoint_defaults_to_the_regional_service_and_honours_an_override` |
| No override | wire `endpoint_defaults_to_the_regional_service_and_honours_an_override` |

## cli

| Scenario | Verified by |
|---|---|
| Increment a counter | cli `counter_get_next_rm_output` |
| Join a queue with tags | cli `queue_join_leave_list_get_output` |
| Explicit region and table | cli `explicit_region_and_table_are_echoed` |
| Local endpoint | cli `counter_get_next_rm_output` (all integration CLI tests use `AWS_ENDPOINT_URL`) |
| First run in a fresh account | cli `counter_get_next_rm_output` (each test creates a fresh table) |
| Get output | cli `counter_get_next_rm_output` |
| Next output | cli `counter_get_next_rm_output` |
| Join output | cli `queue_join_leave_list_get_output` |
| Leave output | cli `queue_join_leave_list_get_output` |
| List output | cli `queue_join_leave_list_get_output` |
| Extract a member ID for scripting | cli `join_output_pipes_to_a_single_counter` |
| Missing id | cli `missing_id_prints_error_and_help_and_exits_1` |
| Missing process on join | cli `missing_process_on_join_exits_1` |
| Malformed tag | cli `malformed_tag_fails_before_contacting_dynamodb` |
| Tag with an equals sign in the value | cli `tag_value_may_contain_equals`; `tests::tags_split_on_first_equals` |
| No subcommand | cli `no_subcommand_at_either_level_exits_1`, `unrecognised_subcommand_exits_1_with_help` |
| Not-found on get | cli `not_found_get_exits_1` |
| Counter command on a queue row | cli `counter_command_on_queue_row_exits_1` |
| Unreachable endpoint | cli `unreachable_endpoint_exits_1_with_connection_error` |
| Quiet by default | cli `logging_goes_to_stderr_only` |
| Verbose | cli `logging_goes_to_stderr_only` |
| Version flag | cli `version_flag_prints_crate_version` |
| Help flag | cli `help_names_program_and_subcommands`, `tag_has_no_short_form` |

## ci

These scenarios describe workflow behaviour. They are verified by the workflow
definitions and by observing runs on GitHub.

| Scenario | Verified by |
|---|---|
| Formatting violation | ci `ci.yml` lint job, `cargo fmt --all --check` |
| Compiler warning | ci `ci.yml`, `RUSTFLAGS=-D warnings` on build and test |
| Unfixable advisory | ci `ci.yml` audit job, `continue-on-error: true` |
| Accepted advisory | `deny.toml` `advisories.ignore` (empty today) |
| MSRV regression | ci `ci.yml` msrv job |
| Newer dependencies in the lock file | ci `ci.yml` msrv job, fallback resolution; `.cargo/config.toml` |
| Fresh fork | ci `ci.yml` uses no secrets |
| Endpoint not reachable | ci `MONOTONE_REQUIRE_INTEGRATION=1`; locally confirmed all 17 it tests fail without an endpoint |
| Weekly run | ci `audit.yml` schedule |
| Version mismatch | ci `release.yml` version job |
| Successful release | ci `release.yml` publish and github-release jobs |
| Rehearsal tag | ci `release.yml`, prerelease path |
| Key pushed by mistake | GitHub push protection (repository setting) |
