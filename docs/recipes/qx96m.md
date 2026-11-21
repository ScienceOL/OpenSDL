# JLM QX96M equipment status

The native driver runs on the OpenSDL worker that owns the instrument
connection. Both local and cluster RPCs read the owning engine's inventory.

Add the station to the worker's engine recipe selected by `--config`. The
address below is an example.

~~~yaml
qx96m_instruments:
  - id: jlm-qpcr
    endpoint: 192.168.42.10:6666
    key_file: private/qx96m.key
~~~

Use the configured private IPv4 address and port of the QX96M. Supply its
existing protocol credential in a host-local file containing exactly eight
printable ASCII bytes. Restrict read access to the service account and
administrators. Relative paths resolve against the recipe's directory. Keep the
key outside version control, shell arguments and service logs.

Limit instrument TCP access to the owning worker on a restricted lab segment.
The instrument's legacy wire protocol relies on this network boundary for peer
trust. Cluster access uses the existing mTLS RPCs.

Move the existing observation entry to this recipe when the worker takes
ownership of the connection. If the vendor client or another provider is using
the instrument, coordinate the connection handover before enabling the recipe.
Each station appears offline until the driver validates a successful reply.
Both the monitor and typed reads publish observations to engine inventory.
Snapshots expose station identity and the latest known idle or active state.
The typed read actions are `device_info` and `run_status`.

Rust consumers constructing an exhaustive `OsdlConfig` literal now include
`qx96m_instruments`. Set it to an empty vector when the engine owns no QX96M.
YAML recipes without this field default to an empty list.

The implementation follows the verified QX96M provider in ScienceOL/Liyan at commit
`bc4a6e94648838d3c991b8c8472eaad63bc62121` under
`plugins/qx96m/internal/instrument` and
`plugins/qx96m/internal/provider/projection.go`. The source reports an 8×12
station and uses the connection reply for the reported serial number, firmware,
and display name. The native acceptance fixture exercises this path in
software using a synthetic credential and TCP peer.
