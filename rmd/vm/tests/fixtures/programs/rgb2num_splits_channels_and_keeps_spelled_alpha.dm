/proc/test()
    var/list/light = rgb2num("#f3fffac4")
    var/list/short = rgb2num("#0f8")
    return "[light[1]],[light[2]],[light[3]],[light[4]]|[short[1]],[short[2]],[short[3]]|[length(short)]"
