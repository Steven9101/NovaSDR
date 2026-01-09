#!/bin/bash
cd /opt/novasdr/

killall -s 9 novasdr-server
killall -s 9 rx888_stream

## Files to load 
#FIFO=fifo.fifo

#[ ! -e "$FIFO" ] && mkfifo $FIFO

./rx888_stream/rx888_stream -f ./rx888_stream/SDDC_FX3.img -s 60000000 -g 50 -m low -o - | ./novasdr-server --no-file-log

#exit


