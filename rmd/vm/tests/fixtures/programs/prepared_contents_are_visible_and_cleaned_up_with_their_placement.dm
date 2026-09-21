/obj/container

/obj/empty

/obj/generated
    name = "prepared item"

/obj/generated/New()
    new /obj/generated_child(src)

/obj/generated_child
    name = "nested item"

/datum/demir/test/prepare(atom/target)
    if(istype(target, /obj/container))
        new /obj/generated(target)

/datum/demir/test/bake(atom/target)
    return

/datum/demir/test/ui(atom/target)
    if(!imgui_begin("Contents"))
        imgui_end()
        return
    imgui_text("[length(target.contents)]")
    for(var/atom/content in target.contents)
        imgui_text("[content.name]")
    imgui_end()
