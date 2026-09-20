// Return the editor dockspace for imgui_set_next_window_dock().
/proc/imgui_dockspace()
	set __demir_intrin = 700

/proc/imgui_set_next_window_dock(dockspace)
	set __demir_intrin = 701

/proc/imgui_set_next_window_size(width, height)
	set __demir_intrin = 702

/proc/imgui_begin(label)
	set __demir_intrin = 703

/proc/imgui_end()
	set __demir_intrin = 704

/proc/imgui_text(text)
	set __demir_intrin = 705

/proc/imgui_text_colored(color, text)
	set __demir_intrin = 706

/proc/imgui_button(label)
	set __demir_intrin = 707

/proc/imgui_checkbox(label, checked)
	set __demir_intrin = 708

/proc/imgui_radio(label, active)
	set __demir_intrin = 717

/proc/imgui_slider(label, value, min, max)
	set __demir_intrin = 709

/proc/imgui_drag(label, value, speed = 1, min = 0, max = 0)
	set __demir_intrin = 710

/proc/imgui_input_text(label, value)
	set __demir_intrin = 711

/proc/imgui_separator(label)
	set __demir_intrin = 712

/proc/imgui_same_line()
	set __demir_intrin = 713

/proc/imgui_tree(label)
	set __demir_intrin = 714

/proc/imgui_tree_end()
	set __demir_intrin = 715

/proc/imgui_collapsing_header(label)
	set __demir_intrin = 716
