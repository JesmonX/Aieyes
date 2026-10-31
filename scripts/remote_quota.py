"""Read Claude OAuth usage on the source host; return only quota fields."""
import json
import os
import sys
import urllib.error
import urllib.request


FIELDS = ('five_hour', 'seven_day', 'seven_day_sonnet', 'seven_day_opus', 'seven_day_oauth_apps', 'extra_usage', 'limits')


def quota_fields(data):
    """Keep only fields that core's Claude normalize reads (windows, limits[], extra_usage)."""
    return {key: data[key] for key in FIELDS if key in data}


def main():
    try:
        root = os.path.expanduser(sys.argv[1])
        with open(os.path.join(root, '.credentials.json'), encoding='utf-8') as f:
            token = json.load(f)['claudeAiOauth']['accessToken']
    except (OSError, KeyError, ValueError):
        return {'error': '未找到远程 Claude Code 登录信息'}
    request = urllib.request.Request('https://api.anthropic.com/api/oauth/usage', headers={
        'Authorization': 'Bearer ' + token, 'anthropic-beta': 'oauth-2025-04-20',
    })
    try:
        with urllib.request.urlopen(request, timeout=25) as response:
            data = json.load(response)
        return quota_fields(data)
    except urllib.error.HTTPError as error:
        if error.code == 401:
            # Claude refreshes its own login on the next run; never refresh it here.
            return {'error': 'Claude Code 登录已过期，打开 Claude Code 一次即可刷新'}
        return {'error': '额度查询失败（HTTP %d）' % error.code}
    except (OSError, ValueError):
        return {'error': '远程额度查询失败，请检查登录状态和网络'}


if __name__ == '__main__':
    print(json.dumps(main(), ensure_ascii=False))
