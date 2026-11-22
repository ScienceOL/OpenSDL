# OpenSDL 集群部署

OpenSDL 集群通过局域网或已有的路由型 VPN 把多台设备主机接到同一个控制入口。工作节点在本机运行 OpenSDL 引擎，持有设备连接和本地事件日志。控制面按工作节点 ID 转发请求，设备 ID 可以在不同工作节点上重复。

## 构建和部署 labadm

每个操作系统和架构构建一份 labadm。集群角色由子命令选择，控制面运行 serve，工作节点运行 join，管理操作和设备查询也使用同一份可执行文件。

~~~sh
cargo build --release --locked -p lab-cli --bin labadm
~~~

安装 target/release/labadm，Windows 对应 target/release/labadm.exe。同一台主机兼任控制面和工作节点时，两项服务指向同一个绝对可执行路径，各自使用对应的 YAML 配置。升级时为同一主机上的两个角色选用同一个版本。仪器接入代码随 OpenSDL 构建发布，调试主机使用仓库产物和本机设备配置。

## 准备网络和证书

控制面需要访问所有工作节点的 TCP 端口和 etcd 客户端端口。操作终端只需访问控制面端口。工作节点监听 LAN 地址或 VPN 地址。跨网段部署时，先通过现有 VPN 打通到工作节点的路由，再用该 VPN 地址注册。

单机开发可用单成员 etcd。高可用部署将三个 etcd 成员放在三个独立故障域，为客户端和成员间通信开启 TLS，并分别部署至少两个控制面副本。所有控制面副本使用同一组 etcd 端点。etcd 负责复制目录和维护仲裁。生产部署配置和快照恢复请参阅 [etcd 运维指南](https://etcd.io/docs/v3.6/op-guide/)。

每个集群使用独立的信任根。PEM 私钥由对应的服务账户保管，签发根私钥留在签发环境。证书按下表签发，客户端和服务端使用不同用途的叶证书。

| 签发根 | 证书用途 | 名称 |
| --- | --- | --- |
| 操作员 CA | 操作终端 clientAuth | 每个操作员一个凭据 |
| 控制面 CA | 控制面 serverAuth | SAN 包含操作终端使用的入口 DNS 名或 IP |
| 控制面 CA | 控制面到工作节点 clientAuth | 每个控制面副本一个凭据 |
| 工作节点 CA | 节点 serverAuth | 精确 DNS SAN 为 WORKER.CLUSTER.worker.opensdl.internal |
| etcd CA | etcd serverAuth 和管理或读取用 clientAuth | 服务端 SAN 与 etcd URL 一致 |

工作节点证书的 DNS SAN 用于校验身份。控制面实际连接的是注册的 IP 和端口，因此不需要为这个 SAN 创建 DNS 记录。每个节点签发自己的证书和密钥。

## 初始化并接入节点

下面示例的集群名为 lab-a，首台控制面为 192.168.80.10。各 YAML 中的证书相对路径以 YAML 文件所在目录为基准。每个副本使用自己的 listen 和叶证书。

~~~yaml
# control.yaml
cluster: lab-a
listen: "192.168.80.10:7443"
incoming:
  certificate: control-a.crt
  private_key: control-a.key
  ca: operator-ca.crt
workers:
  certificate: control-a-client.crt
  private_key: control-a-client.key
  ca: worker-ca.crt
directory:
  endpoints:
    - "https://kv-a.lab.example:2379"
    - "https://kv-b.lab.example:2379"
    - "https://kv-c.lab.example:2379"
  tls:
    certificate: directory-reader.crt
    private_key: directory-reader.key
    ca: etcd-ca.crt
~~~

启用 etcd 按证书身份授权时，把服务配置的目录凭据限制为对应集群前缀 /opensdl/v1/lab-a/ 的读权限。管理员使用同结构的 admin.yaml，其中目录凭据有该前缀的读写权限。运行一次 init，在每台控制面主机分别运行 serve。

~~~sh
labadm --cluster-config admin.yaml init
labadm --cluster-config control.yaml serve
~~~

需要从 LiyanLabs 接入时，在控制面服务账户下为该控制面实例执行账号绑定。将下列配置加入 `control.yaml` 后启动现有的 `labadm serve`。此监听复用集群目录和节点证书路由。

~~~yaml
account_gateway:
  listen: "100.64.0.10:50051"
  instance: lab-a
  media_relay: "rtmps://srs.test.liyanlabs.com:11935"
~~~

~~~sh
lab --instance lab-a login --server https://test.liyanlabs.com
~~~

在同环境的 LiyanLabs 中选择同一账号连接，填写上述地址。账号入口只允许回环或显式配置的 Tailscale IPv4 地址。设备使用 `工作节点:设备 ID` 寻址，命令、事件和摄像头发布都转发到所属工作节点。摄像头只发布到配置的本环境 RTMPS 中继，省略 `media_relay` 时停用该入口的发布能力。单次命令只派发一次，传输中断后的命令结果须到设备端确认。

账号入口支持最多 128 个工作节点和每副本 8 个并发事件订阅。设备清单使用一次一致性读取获取成员，保留可达节点的结果；`lab status` 只探测目录就绪，实际设备数以 `lab device list` 为准。事件订阅建立时读取成员，已有节点断线会独立恢复，首次加入的成员在客户端重连后可订阅。

工作节点示例使用 worker-01.lab-a.worker.opensdl.internal 作为证书 SAN。

~~~yaml
# worker.yaml
cluster: lab-a
worker: worker-01
listen: "192.168.80.21:7444"
incoming:
  certificate: worker-01.crt
  private_key: worker-01.key
  ca: control-ca.crt
~~~

先在节点上运行仿真，用相同的命令入口验证集群。接入物理设备时，使用该节点的内置驱动配置及私有数据目录，并通过进程管理器运行 join。每台物理设备只由它实际连接的工作节点管理。

工作节点要求显式传入 --config 或 --simulation。设备配方使用 MQTT 时，将 mqtt.host 设置为工作节点的仪器网卡 IPv4 地址。嵌入式 MQTT 只绑定该地址，使用既有设备协议。将该网卡置于仪器隔离网段，通过固件的 MQTT_HOST 指向此节点，并在防火墙上将 MQTT 端口限制给仪器网段。工作节点仅在 gRPC 端口运行双向 TLS，MQTT 与视频传输是独立设备链路。

~~~sh
labadm --cluster-config worker.yaml join --simulation --data-dir ./worker-01-data
# 连接实际设备时使用设备配方和本地设备注册表
labadm --cluster-config worker.yaml join --config ./devices.yaml --registry ./registry/unilabos --data-dir ./worker-01-data
~~~

如果这台主机还需要让桌面插件直接连接，用 `lab serve --cluster-worker` 启动同一个设备引擎。保留现有实例名、设备配方和状态目录，在工作节点 YAML 中保留集群身份、证书和监听地址。桌面插件继续使用实验室的账户或配对认证端口，控制面使用工作节点的双向 TLS 端口。

~~~sh
lab login --server https://test.liyanlabs.com --instance laboratory
lab serve --instance laboratory --auth account --listen 100.64.0.9:50051 \
  --config ./devices.yaml --data-dir ./laboratory-data --cluster-worker ./worker.yaml
~~~

从两进程迁移时，停止原来的 `lab serve` 和同主机的 `labadm join`，再启动上面的服务。保留工作节点原来已登记的地址和证书，控制面会继续使用同一条目录记录。相机视频源、设备连接、事件日志和推流进程由共享引擎持有。

管理员从能访问该工作节点的主机完成接入。admit 会先验证节点证书和引擎身份，再提交注册。所有副本读取同一条记录。迁移仪器前先停止旧的工作节点，通过 forget 移除旧路由，再以新节点接入。

~~~sh
labadm --cluster-config admin.yaml admit worker-01 192.168.80.21:7444
~~~

### 通过内置 TCP 探测登记只读设备

在工作节点的 `worker.yaml` 中添加 `read_only_devices`。节点启动后即列出这些设备，并在后台每 30 秒探测一次。探测失败时设备仍在目录中，状态为离线。目录和设备详情查询直接读取最近的探测结果。只读设备的动作列表为空。

```yaml
read_only_devices:
  - id: network-instrument
    device_type: vendor.instrument
    description: Instrument TCP endpoint reachability
    probe:
      type: tcp
      address: 192.168.80.100:6666
```

`tcp` 建立 TCP 连接后关闭。在线表示指定端口可达，设备详情会包含 `tcp_reachable`。仪器身份和实验运行状态需要由本机驱动确认。

Linux 工作节点可通过 `v4l2_depth_camera` 登记本机 USB 深度相机。使用设备的
`/dev/v4l/by-id` 或 `/dev/v4l/by-path` 路径，并确保运行工作节点的用户有读取权限。

~~~yaml
read_only_devices:
  - id: depth-camera
    device_type: v4l2.depth_camera
    description: USB depth camera
    probe:
      type: v4l2_depth_camera
      depth_device: /dev/v4l/by-id/usb-depth-camera-video-index0
      color_device: /dev/v4l/by-path/usb-color-camera-video-index0
~~~

工作节点确认深度接口提供 `Z16` 时标记相机在线。可选的彩色接口会分别报告
`color_available`。这个探测只读取相机能力，实时视频仍由媒体源配置提供。

## 操作设备

~~~yaml
# operator.yaml
cluster: lab-a
endpoints:
  - "https://192.168.80.10:7443"
  - "https://192.168.80.11:7443"
tls:
  certificate: operator.crt
  private_key: operator.key
  ca: control-ca.crt
~~~

~~~sh
labadm --cluster-config operator.yaml status
labadm --cluster-config operator.yaml workers
labadm --cluster-config operator.yaml devices
labadm --cluster-config operator.yaml device worker-01 sim:lab-sim:heater-1
labadm --cluster-config operator.yaml send worker-01 sim:lab-sim:heater-1 set_temperature --params '{"temperature":30}'
labadm --cluster-config operator.yaml events worker-01 --kind command_result
~~~

操作员也可以通过一个支持 HTTP/2 透传的入口访问多个控制面副本。入口需保留双向 TLS 身份校验，并仅公布能访问共享目录的副本。客户端可调用 osdl.cluster.v1.ControlPlane/Ready 确认副本与目录就绪。

控制面故障后，新的操作会选择可用副本。命令在传输中断时可能已经到达设备。请先在原工作节点核对仪器的实际状态。拥有该主机访问权限的管理员可以用命令 ID 在该节点数据目录的 WORKER.db 中查询 command_log，确认引擎是否收到派发请求。日志记录派发请求，仪器是否完成动作仍需核实。命令 ID 用于关联日志。再次发出相同 ID 会产生新的派发。工作节点离线时仍出现在清单中，其设备不可操作。控制面与 etcd 多数派失联时，目录操作失败。现有事件流在断开后需要重连，重连从当时的实时事件开始。

运行本地故障演练需要 Docker 和 OpenSSL。验收使用仿真设备，覆盖三成员 etcd、控制面切换、节点离线、错误证书和断线时的命令唯一派发。

~~~sh
cargo build --locked -p lab-cli
python3 scripts/test-cluster.py
~~~

[架构与数据边界](architecture/cluster.md)
