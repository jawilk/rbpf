/**
 * @brief test program
 */

typedef unsigned char uint8_t;
typedef unsigned long int uint64_t;

extern void log(const char*, uint64_t);
extern void log_64(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t);

uint8_t foo_test(uint8_t a, uint64_t lo, uint64_t hi, uint8_t item)
{
    int i = 34;
    i += a;
    return i;
}

extern uint64_t entrypoint(const uint8_t *input) {
  char* abcd = "HELLO";
  int blaaaah = 9999;
  foo_test(1,2,3,4);
  log(__func__, sizeof(__func__));
  log_64(1, 2, 3, 4, 5);
  return 0;
}
