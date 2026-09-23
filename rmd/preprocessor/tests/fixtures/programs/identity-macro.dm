#define IDENTITY(x) x
#define PATH(_NAME) datum/namespace/##_NAME/##IDENTITY
PATH(CHEM)(var/const/REQUEST = 1)
