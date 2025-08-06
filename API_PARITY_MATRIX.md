# 📊 API Parity Matrix

| Function | Node.js | WASM | Python | Status |
|----------|---------|------|--------|--------|
| `add_sheet` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `apply_external_diffs` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `auto_fill_columns` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `auto_fill_rows` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `can_redo` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `can_undo` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `clear_cell_contents` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `column_name_from_number` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `column_name_from_number_js` | ✅ | ❌ | ❌ | ❌ LIMITED |
| `copy_to_clipboard` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `create` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `create_user_model` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `create_user_model_from_bytes` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `create_user_model_from_icalc` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `create_user_model_from_xlsx` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `delete_columns` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `delete_defined_name` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `delete_definedname` | ✅ | ✅ | ❌ | ⚠️ PARTIAL |
| `delete_rows` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `delete_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `evaluate` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `flush_send_queue` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `from_bytes` | ✅ | ❌ | ❌ | ❌ LIMITED |
| `from_icalc` | ✅ | ❌ | ❌ | ❌ LIMITED |
| `from_xlsx` | ✅ | ❌ | ❌ | ❌ LIMITED |
| `get_cell_content` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_cell_style` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_cell_type` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_column_width` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_columns_with_data` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `get_defined_name_list` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_first_non_empty_in_row_after_column` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `get_formatted_cell_value` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_frozen_columns_count` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_frozen_rows_count` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_last_non_empty_in_row_before_column` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `get_name` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_row_height` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_rows_with_data` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `get_scroll_x` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_scroll_y` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_selected_cell` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_selected_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_selected_view` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `get_show_grid_lines` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_tokens` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_worksheets_properties` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `hide_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `insert_columns` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `insert_rows` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `load_from_bytes` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `load_from_icalc` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `load_from_xlsx` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `move_column` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `move_column_action` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `move_row` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `move_row_action` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `new` | ✅ | ✅ | ❌ | ⚠️ PARTIAL |
| `new_defined_name` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `new_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_area_selecting` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_arrow_down` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_arrow_left` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_arrow_right` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_arrow_up` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_expand_selected_range` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_navigate_to_edge_in_direction` | ❌ | ✅ | ✅ | ⚠️ PARTIAL |
| `on_page_down` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_page_up` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `on_paste_styles` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `paste_csv_string` | ✅ | ✅ | ❌ | ⚠️ PARTIAL |
| `paste_csv_text` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `paste_from_clipboard` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `pause_evaluation` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `range_clear_all` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `range_clear_contents` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `range_clear_formatting` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `redo` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `rename_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `resume_evaluation` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `save_to_icalc` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `save_to_xlsx` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `set_area_with_border` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_cell_style` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `set_column_width` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `set_columns_width` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_frozen_columns_count` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_frozen_rows_count` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_name` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_row_height` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `set_rows_height` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_selected_cell` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_selected_range` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_selected_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_sheet_color` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_show_grid_lines` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_top_left_visible_cell` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_user_input` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_window_height` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `set_window_width` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `test_panic` | ❌ | ❌ | ✅ | ❌ LIMITED |
| `to_bytes` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `undo` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `unhide_sheet` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `update_defined_name` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `update_range_style` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |