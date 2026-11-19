# OpenSDL cluster architecture

A cluster has a shared directory, interchangeable control-plane replicas, and
work-plane nodes. Each worker runs the existing OpenSDL engine against its own
configuration, hardware, and event store. A device is addressed by the pair
(worker, device ID). Two workers may report identical local IDs. A command follows
one authorized route to the owning engine and returns its normal dispatch result.

A dedicated etcd v3 quorum holds an initialization marker and one record per
approved worker. The operator initializes the cluster, starts one or more
control-plane replicas, starts work-plane agents and admits their fixed network
addresses. Replicas read the same linearizable directory. Membership writes use
transactions. Targeted calls read a single record and reuse gRPC channels.
Inventory reads device summaries with bounded parallelism and reports errors
per worker. Device details and action schemas are retrieved by exact ID.
Availability is observed on demand from the owning service. A device's actions
and event history live on its worker.

Mutual TLS uses separate trust roots for operators, controls, worker servers,
and etcd. Worker TLS names bind the cluster and worker ID to the registered
address. Work-plane ingress trusts only the control-plane role. Management of
membership requires direct administrative access to etcd; operator access to
device RPCs grants no right to change routes.

Clients select a ready control endpoint before initiating an operation. After a
command is sent, a transport failure leaves its outcome unknown. On the owning
host, the local SQLite command_log can correlate the command ID with a dispatch
attempt. Operators verify the instrument state to determine the physical
outcome. Command IDs are correlation identifiers, and a new submission using
the same ID dispatches again. A fresh client
may select another replica for subsequent operations. Network or quorum loss
makes affected operations unavailable. Worker failure leaves its devices
unavailable until that worker returns. Physical ownership changes require
explicit operator action.

The Kubernetes reference is kubernetes/kubernetes v1.34.1 at
93248f9ae092f571eb870b7664c534bfc7d00f03. The kubeadm init/join phase runners
separate bootstrap from supervised processes. The control-plane manifest
implementation selects external etcd endpoints. The API server installs the
Node REST storage and kubelet reports independent node health via its lease
controller and the node-lifecycle controller. OpenSDL uses the external quorum
and interchangeable API endpoints, with explicit instrument ownership and
on-demand worker probes. It needs no scheduler, periodic heartbeat writes or
workload rescheduling.

The etcd implementation reference is etcd-io/etcd v3.6.5 at
a0614505aff9b8ff469e9c59c2d979a5936d13f4. The directory uses its native v3
gRPC Range and Txn contracts. Exact reads use the default linearizable
path. Prefix pages use default ascending key order so the range limit is
applied within the MVCC storage path.
