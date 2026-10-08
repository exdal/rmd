/datum/demir/test
    modular_loader = DEMIR_MODULAR_LOADER_TG

    New()
        ..()
        modular_loader = "constructor"

    prepare(atom/target)
        modular_loader = null
