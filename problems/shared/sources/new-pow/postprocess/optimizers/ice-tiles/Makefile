CC ?= gcc
CFLAGS ?= -O3 -march=native -Wall -Wextra -fPIC -shared

all: libicetiles.so libgreedy.so

libicetiles.so: icetiles.c
	$(CC) $(CFLAGS) -o $@ $< -lm

libgreedy.so: greedy.c
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -f libicetiles.so libgreedy.so

.PHONY: all clean
