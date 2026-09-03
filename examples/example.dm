#define MAX_DAMAGE 100

#if MAX_DAMAGE > 0
    #warn "nested preprocessors"
#endif

/obj/item
	name = "item"
	icon = 'icons/obj/items.dmi'
	var/damage = 0
	var/mob/living/owner = null

/obj/item/sword
	name = "sword"
	icon_state = "sword"
	damage = 25

	proc/swing(mob/living/target)
		if(!target)
			return
		target.take_damage(min(damage, MAX_DAMAGE))
		to_chat(target, "[src] hits you for [damage] damage!")

/turf/open/floor
	name = "floor"
	icon = 'icons/turf/floors.dmi'
	icon_state = "floor"
