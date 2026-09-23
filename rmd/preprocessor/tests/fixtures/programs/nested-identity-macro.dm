#define IDENTITY(x) x
#define ADD_1(a) datum/namespace/##a/##IDENTITY
#define ADD_2(a, b) datum/namespace/inner/##IDENTITY
#define CREATE(a, b) ADD_1(a)(var/##ADD_2(a, b)(##b = 1))
CREATE(CHEM, REQUEST)
