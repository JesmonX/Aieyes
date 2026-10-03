"""Read Claude OAuth usage on the source host; return only quota fields."""
import json
import os
import sys
import urllib.error
import urllib.request


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
        return {key: data[key] for key in ('five_hour', 'seven_day', 'seven_day_sonnet', 'seven_day_opus', 'seven_day_oauth_apps') if key in data}
    except urllib.error.HTTPError as error:
        return {'error': '限额查询失败（HTTP %d）' % error.code}
    except (OSError, ValueError):
        return {'error': '远程限额连接失败，请检查前置命令中的代理设置'}


if __name__ == '__main__':
    print(json.dumps(main(), ensure_ascii=False))
