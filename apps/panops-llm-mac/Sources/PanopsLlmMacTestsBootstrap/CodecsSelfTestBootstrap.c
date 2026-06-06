extern void panops_codecs_self_test_run(void);

__attribute__((constructor))
static void panops_codecs_self_test_bootstrap(void) {
    panops_codecs_self_test_run();
}
