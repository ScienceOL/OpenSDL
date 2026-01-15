package events

import (
	// 外部依赖
	"context"
	"encoding/json"
	"sync"
	"time"

	r "github.com/redis/go-redis/v9"
	
	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	notify "github.com/scienceol/opensdl/service/pkg/core/notify"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

/*
	使用 redis 的发布订阅实现多进程间广播通信
*/

var (
	once   sync.Once
	center *Events
)

type Events struct {
	actions sync.Map
	subs    sync.Map
	client  *r.Client
	wait    sync.WaitGroup
}

func NewEvents() notify.MsgCenter {
	once.Do(func() {
		center = &Events{
			actions: sync.Map{},
			client:  redis.GetClient(),
			wait:    sync.WaitGroup{},
		}
	})

	return center
}

func (e *Events) Registry(ctx context.Context, msgName notify.Action, handleFunc notify.HandleFunc) error {
	if _, ok := e.actions.LoadOrStore(msgName, handleFunc); ok {
		return code.NotifyActionAlreadyRegistryErr.WithMsg(string(msgName))
	}

	// 订阅消息
	sub := e.client.Subscribe(ctx, string(msgName))
	e.subs.Store(msgName, sub)

	e.wait.Add(1)
	utils.SafelyGo(func() {
		defer e.wait.Done()

		ch := sub.Channel()
		for {
			select {
			case msg, ok := <-ch:
				if !ok {
					logger.Infof(ctx, "exit redis channel name: %s", string(msgName))
					if err := sub.Unsubscribe(ctx, string(msgName)); err != nil {
						logger.Errorf(ctx, "unsubscribe fail msg name: %s, err: %+v", msgName, err)
					}
					e.actions.Delete(msgName)
					return
				}

				if msg == nil {
					continue
				}
				if err := handleFunc(ctx, msg.Payload); err != nil {
					logger.Errorf(ctx, "handle redis msg fail name: %s, err: %+v", msgName, err)
				}
			case <-ctx.Done():
				logger.Infof(ctx, "exit redis channel name: %s", string(msgName))
				if err := sub.Unsubscribe(ctx, string(msgName)); err != nil {
					logger.Errorf(ctx, "unsubscribe fail msg name: %s, err: %+v", msgName, err)
				}
				e.actions.Delete(msgName)
				return
			}
		}
	}, func(err error) {
		logger.Errorf(ctx, "Registry handle msg err: %+v", err)
	})
	return nil
}

func (e *Events) Broadcast(ctx context.Context, msg *notify.SendMsg) error {
	msg.Timestamp = time.Now().Unix()
	if msg.UUID.IsNil() {
		msg.UUID = uuid.NewV4()
	}

	data, _ := json.Marshal(msg)
	ret := e.client.Publish(ctx, string(msg.Channel), data)
	if ret.Err() != nil {
		logger.Errorf(ctx, "send msg fail action: %s, err: %+v", msg.Channel, ret.Err())
		return code.NotifySendMsgErr
	}

	return nil
}

func (e *Events) Close(ctx context.Context) error {
	e.wait.Wait()
	return nil
}
