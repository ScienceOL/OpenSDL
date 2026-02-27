package casdoor

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"

	"github.com/go-resty/resty/v2"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/repo"
	"github.com/scienceol/osdl/pkg/repo/model"
)

type casClient struct {
	client *resty.Client
}

func NewCasClient() repo.Account {
	conf := config.Global().OAuth2
	return &casClient{
		client: resty.New().
			SetBaseURL(conf.Addr).
			SetBasicAuth(conf.ClientID, conf.ClientSecret),
	}
}

func (c *casClient) GetLabUserInfo(ctx context.Context, aksk *model.LabAkSk) (*model.UserData, error) {
	resp, err := c.client.R().SetContext(ctx).
		SetQueryParams(map[string]string{
			"accessKey":    aksk.AccessKey,
			"accessSecret": aksk.AccessSecret,
		}).
		Get("/api/get-account")
	if err != nil {
		logger.Errorf(ctx, "GetLabUserInfo http err: %+v", err)
		return nil, code.CasDoorQueryLabUserErr
	}
	if resp.StatusCode() != http.StatusOK {
		return nil, code.CasDoorQueryLabUserErr.WithMsgf("http code: %d", resp.StatusCode())
	}
	result := &model.UserInfo{}
	if err := json.Unmarshal(resp.Body(), result); err != nil || result.Status != "ok" || result.Data == nil {
		return nil, code.CasDoorQueryLabUserErr
	}
	return result.Data, nil
}

func (c *casClient) GetLabUserByAccessKey(ctx context.Context, accessKey string) (*model.UserData, error) {
	resp, err := c.client.R().SetContext(ctx).
		SetQueryParam("accessKey", accessKey).
		Get("/api/get-account")
	if err != nil {
		logger.Errorf(ctx, "GetLabUserByAccessKey http err: %+v", err)
		return nil, code.CasDoorQueryLabUserErr
	}
	if resp.StatusCode() != http.StatusOK {
		return nil, fmt.Errorf("casdoor query failed: %d", resp.StatusCode())
	}
	result := &model.UserInfo{}
	if err := json.Unmarshal(resp.Body(), result); err != nil || result.Status != "ok" || result.Data == nil {
		return nil, code.CasDoorQueryLabUserErr
	}
	return result.Data, nil
}
