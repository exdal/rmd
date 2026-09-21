
/proc/test(mob/user, amount = 1 as num in 1 to 10, ...)
    var/list/items[5], other = list()
    set waitfor = 0
    if(!user)
        return
    else if(amount > 2)
        world << "hi"
    else
        user >> amount
    while(amount)
        amount--
    do
        amount++
    while(amount < 2)
    spawn(1)
        del user
    try
        throw user
    catch(/datum/error/e)
        goto done
    obj:field()
    done:
        break
