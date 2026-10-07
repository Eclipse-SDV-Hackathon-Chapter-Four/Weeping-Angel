#!/usr/bin/bash

FOUND=NO

for i in `pidof dfm_sovd_bridge` ; do
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1
done

if [[ $FOUND == YES ]] ; then
    echo "DFM SOVD bridge killed"
else
    echo "No DFM SOVD bridge process found"
fi
