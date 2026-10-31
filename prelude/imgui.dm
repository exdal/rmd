// Next available intrinsic index: 733 (shared by demir.dm and imgui.dm).
// PLEASE UPDATE THIS WHEN YOU ADD NERW ID!!!

// Return the editor dockspace for imgui_set_next_window_dock().
/proc/imgui_dockspace()
	set __demir_intrin = "ImguiDockspace"

/proc/imgui_set_next_window_dock(dockspace)
	set __demir_intrin = "ImguiSetNextWindowDock"

/proc/imgui_set_next_window_size(width, height)
	set __demir_intrin = "ImguiSetNextWindowSize"

/proc/imgui_begin(label)
	set __demir_intrin = "ImguiBegin"

/proc/imgui_end()
	set __demir_intrin = "ImguiEnd"

/proc/imgui_text(text)
	set __demir_intrin = "ImguiText"

/proc/imgui_text_colored(color, text)
	set __demir_intrin = "ImguiTextColored"

/proc/imgui_button(label)
	set __demir_intrin = "ImguiButton"

/proc/imgui_checkbox(label, checked)
	set __demir_intrin = "ImguiCheckbox"

/proc/imgui_radio(label, active)
	set __demir_intrin = "ImguiRadio"

/proc/imgui_slider(label, value, min, max)
	set __demir_intrin = "ImguiSlider"

/proc/imgui_drag(label, value, speed = 1, min = 0, max = 0)
	set __demir_intrin = "ImguiDrag"

/proc/imgui_input_text(label, value)
	set __demir_intrin = "ImguiInputText"

/proc/imgui_separator(label)
	set __demir_intrin = "ImguiSeparator"

/proc/imgui_same_line()
	set __demir_intrin = "ImguiSameLine"

/proc/imgui_tree(label)
	set __demir_intrin = "ImguiTree"

/proc/imgui_tree_end()
	set __demir_intrin = "ImguiTreeEnd"

/proc/imgui_collapsing_header(label)
	set __demir_intrin = "ImguiCollapsingHeader"

/proc/imgui_mouse_popup()
	set __demir_intrin = "ImguiMousePopup"

/proc/imgui_open_popup(popup)
	set __demir_intrin = "ImguiOpenPopup"

/proc/imgui_begin_popup(popup)
	set __demir_intrin = "ImguiBeginPopup"

/proc/imgui_end_popup()
	set __demir_intrin = "ImguiEndPopup"
