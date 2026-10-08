/datum/demir/loader
    default = TRUE
    modular_loader = DEMIR_MODULAR_LOADER_TG

/datum/demir/loader/inherited

/datum/demir/loader/disabled
    modular_loader = null

/datum/demir/loader/custom
    modular_loader = "custom-loader"

/datum/demir/loader/empty
    modular_loader = ""

/datum/demir/no_loader

/datum/demir/loader/constructor_enabled
    modular_loader = null

    New()
        ..()
        modular_loader = DEMIR_MODULAR_LOADER_TG

/datum/demir/loader/constructor_disabled
    New()
        ..()
        modular_loader = null
