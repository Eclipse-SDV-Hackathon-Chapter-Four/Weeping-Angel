#!/usr/bin/bash

FOUND=NO

for i in `pidof databroker` ; do
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1
done

if [[ $FOUND == YES ]] ; then
    echo "KUKSA data broker killed"
else
    echo "No KUKSA data broker process found"
fi
