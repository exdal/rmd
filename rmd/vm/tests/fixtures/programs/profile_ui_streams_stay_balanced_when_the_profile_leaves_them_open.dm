
/datum/demir/test/ui(atom/target)
    imgui_begin("Panel")
    if(imgui_tree("Nested"))
        imgui_text("only when open")
        imgui_tree_end()
    imgui_text("after")
