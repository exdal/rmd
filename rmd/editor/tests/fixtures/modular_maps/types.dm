/obj/modular_map_root
    var/config_file = null
    var/key = null

/obj/modular_map_root/fixture
    config_file = "config.toml"

/obj/modular_map_root/fixture/inherited
    key = "leaf"

// Verify DM parent_type relationships, rather than path-prefix matching.
/obj/root_alias
    parent_type = /obj/modular_map_root/fixture

/obj/modular_map_connector

/obj/connector_alias
    parent_type = /obj/modular_map_connector
