#!/bin/bash

veths=$(ip link show | grep @ | cut -d ' ' -f 2 | cut -d '@' -f 1)
while IFS= read -r veth; do
    echo "removing $veth"
    sudo ip link delete dev $veth
done <<< "$veths"
